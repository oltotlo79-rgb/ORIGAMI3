//! Browserless acceptance measurements for the rigid surface-order contract.
//!
//! The raster path intentionally follows Viewer3D's Float32 projection,
//! 24-bit depth target, two-code depth tolerance, and surface-owner ordering.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, Neg, Sub};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use ori3_cp::{Face, extract_faces};
use ori3_model::{
    CreasePattern, Document, Driver, Edge, EdgeId, EdgeKind, FaceId, Frame3D, Vertex,
};
use ori3_rigid::{propagate, solve_motion, to_frame3d};

const CAMERA_FOV_DEG: f64 = 45.0;
const CAMERA_MARGIN: f64 = 1.35;
const CAMERA_NEAR: f64 = 0.01;
const CAMERA_FAR: f64 = 100.0;
const DEPTH_BITS: u32 = 24;
const DEPTH_TIE_CODES: u32 = 2;
const VIEWPORT: usize = 800;
const RASTER_EPS: f32 = 1e-10;

const WARMUP_ABS: [f64; 19] = [
    0.0, 9.0, 19.0, 29.0, 39.0, 49.0, 59.0, 69.0, 79.0, 90.0, 101.0, 111.0, 121.0, 131.0, 141.0,
    151.0, 161.0, 171.0, 179.0,
];
const BOUNDARY_ABS: [f64; 5] = [179.5, 179.9, 179.99, 179.999, 180.0];

const FOLDED_SAMPLE: &str =
    include_str!("../../../../crates/ori3-layers/tests/fixtures/folded-sample.ori3");

#[derive(Clone, Copy, Debug, Default)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}

impl V3 {
    const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    const Y: Self = Self::new(0.0, 1.0, 0.0);
    const Z: Self = Self::new(0.0, 0.0, 1.0);

    const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    fn length_squared(self) -> f64 {
        self.dot(self)
    }

    fn normalize(self) -> Self {
        self / self.length_squared().sqrt()
    }
}

impl Add for V3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for V3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for V3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<f64> for V3 {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        self * (1.0 / rhs)
    }
}

impl DivAssign<f64> for V3 {
    fn div_assign(&mut self, rhs: f64) {
        *self = *self / rhs;
    }
}

impl Neg for V3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        self * -1.0
    }
}

#[derive(Clone)]
struct Diagram {
    name: &'static str,
    cp: CreasePattern,
    faces: Vec<Face>,
    hinges: Vec<(EdgeId, EdgeKind)>,
    paper_width: f64,
    paper_height: f64,
    triangles: BTreeMap<FaceId, Vec<[usize; 3]>>,
    owner_codes: BTreeMap<FaceId, i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VisualImage {
    visible_faces: BTreeSet<FaceId>,
    visible_back_faces: BTreeSet<FaceId>,
    red_pixels: u64,
    light_pixels: u64,
}

impl VisualImage {
    fn red_ratio(&self) -> f64 {
        let total = self.red_pixels + self.light_pixels;
        if total == 0 {
            0.0
        } else {
            self.red_pixels as f64 / total as f64
        }
    }
}

#[derive(Clone, Copy)]
struct Camera {
    position: V3,
    view_x: [f32; 4],
    view_y: [f32; 4],
    view_depth: [f32; 4],
    projection_scale: f32,
    projection_depth_a: f32,
    projection_depth_b: f32,
}

#[derive(Clone, Copy)]
struct Projected {
    x: f32,
    y: f32,
    depth: f32,
}

struct RenderFace {
    face: FaceId,
    owner_code: i64,
    surface_rank: u32,
    side: i64,
    material_orientation: i64,
    triangles: Vec<RenderTriangle>,
}

struct RenderTriangle {
    projected: [Projected; 3],
    back_facing: bool,
}

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

fn split_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 0.5, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 1.0),
            vertex(4, 0.5, 1.0),
            vertex(5, 0.0, 1.0),
        ],
        edges: vec![
            edge(0, 0, 1, EdgeKind::Border),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 3, EdgeKind::Border),
            edge(3, 3, 4, EdgeKind::Border),
            edge(4, 4, 5, EdgeKind::Border),
            edge(5, 5, 0, EdgeKind::Border),
            edge(6, 1, 4, EdgeKind::Mountain),
        ],
        next_vertex_id: 6,
        next_edge_id: 7,
    }
}

fn flat_foldable_kome() -> CreasePattern {
    let radial_kinds = [
        EdgeKind::Valley,
        EdgeKind::Valley,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Valley,
        EdgeKind::Mountain,
    ];
    let mut edges = vec![
        edge(0, 0, 1, EdgeKind::Border),
        edge(1, 1, 2, EdgeKind::Border),
        edge(2, 2, 3, EdgeKind::Border),
        edge(3, 3, 4, EdgeKind::Border),
        edge(4, 4, 5, EdgeKind::Border),
        edge(5, 5, 6, EdgeKind::Border),
        edge(6, 6, 7, EdgeKind::Border),
        edge(7, 7, 0, EdgeKind::Border),
    ];
    edges.extend(
        radial_kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| edge(8 + index as u32, 8, index as u32, kind)),
    );
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 0.5, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 0.5),
            vertex(4, 1.0, 1.0),
            vertex(5, 0.5, 1.0),
            vertex(6, 0.0, 1.0),
            vertex(7, 0.0, 0.5),
            vertex(8, 0.5, 0.5),
        ],
        edges,
        next_vertex_id: 9,
        next_edge_id: 16,
    }
}

fn angle_surface_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 0.0, 0.5),
            vertex(5, 1.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.792_893_218_813_452_5, 0.5),
            vertex(10, 0.5, 0.207_106_781_186_547_52),
            vertex(11, 0.25, 0.5),
            vertex(12, 0.5, 0.792_893_218_813_452_5),
            vertex(13, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 3, 4, EdgeKind::Border),
            edge(5, 4, 0, EdgeKind::Border),
            edge(6, 1, 5, EdgeKind::Border),
            edge(7, 5, 2, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 8, 9, EdgeKind::Mountain),
            edge(20, 9, 5, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 9, 1, EdgeKind::Mountain),
            edge(23, 8, 10, EdgeKind::Mountain),
            edge(24, 10, 7, EdgeKind::Mountain),
            edge(25, 0, 10, EdgeKind::Mountain),
            edge(26, 10, 1, EdgeKind::Mountain),
            edge(28, 11, 8, EdgeKind::Mountain),
            edge(31, 6, 12, EdgeKind::Mountain),
            edge(32, 12, 8, EdgeKind::Mountain),
            edge(33, 2, 12, EdgeKind::Mountain),
            edge(34, 12, 3, EdgeKind::Mountain),
            edge(37, 4, 13, EdgeKind::Mountain),
            edge(38, 13, 11, EdgeKind::Mountain),
            edge(39, 0, 13, EdgeKind::Mountain),
            edge(40, 13, 3, EdgeKind::Mountain),
            edge(42, 13, 12, EdgeKind::Valley),
            edge(43, 10, 9, EdgeKind::Valley),
        ],
        next_vertex_id: 14,
        next_edge_id: 44,
    }
}

fn angle_surface_angles(edge_43: f64) -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (20, -95.0),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -95.0),
        (25, 180.0),
        (31, -123.0),
        (32, 180.0),
        (34, 180.0),
        (37, -123.0),
        (39, 180.0),
        (40, 180.0),
        (42, -57.0),
        (43, edge_43),
    ])
}

fn fold_hinges(cp: &CreasePattern, faces: &[Face]) -> Vec<(EdgeId, EdgeKind)> {
    let mut owners = BTreeMap::<EdgeId, usize>::new();
    for face in faces {
        for &edge_id in &face.edges {
            *owners.entry(edge_id).or_default() += 1;
        }
    }
    cp.edges
        .iter()
        .filter(|item| {
            matches!(item.kind, EdgeKind::Mountain | EdgeKind::Valley)
                && owners.get(&item.id) == Some(&2)
        })
        .map(|item| (item.id, item.kind))
        .collect()
}

fn diagram(name: &'static str, cp: CreasePattern, paper_width: f64, paper_height: f64) -> Diagram {
    let faces = extract_faces(&cp);
    let hinges = fold_hinges(&cp, &faces);
    let triangles = triangulations(&cp, &faces);
    let mut face_ids = faces.iter().map(|face| face.id).collect::<Vec<_>>();
    face_ids.sort_unstable();
    let owner_codes = face_ids
        .into_iter()
        .enumerate()
        .map(|(index, face)| (face, index as i64 + 1))
        .collect();
    Diagram {
        name,
        cp,
        faces,
        hinges,
        paper_width,
        paper_height,
        triangles,
        owner_codes,
    }
}

fn boundary_diagrams() -> Vec<Diagram> {
    let fixture: Document =
        serde_json::from_str(FOLDED_SAMPLE).expect("folded-sample fixture is a Document");
    let long = fixture.paper.width_mm.max(fixture.paper.height_mm);
    let paper_width = fixture.paper.width_mm / long;
    let paper_height = fixture.paper.height_mm / long;
    let diagrams = vec![
        diagram("split-square", split_square(), 1.0, 1.0),
        diagram("diagonal-midline-square", flat_foldable_kome(), 1.0, 1.0),
        diagram("folded-sample.ori3", fixture.cp, paper_width, paper_height),
    ];
    assert_eq!(diagrams[0].hinges.len(), 1);
    assert_eq!(diagrams[1].hinges.len(), 8);
    assert_eq!(diagrams[2].hinges.len(), 101);
    diagrams
}

fn triangulations(cp: &CreasePattern, faces: &[Face]) -> BTreeMap<FaceId, Vec<[usize; 3]>> {
    const SCRIPT: &str = r#"
import fs from 'node:fs';
import * as THREE from 'three';

const polygons = JSON.parse(fs.readFileSync(0, 'utf8'));
const orient = (points, a, b, c) => {
  const cross =
    (points[b][0] - points[a][0]) * (points[c][1] - points[a][1]) -
    (points[b][1] - points[a][1]) * (points[c][0] - points[a][0]);
  return cross < 0 ? [a, c, b] : [a, b, c];
};
const result = polygons.map((points) => {
  const contour = points.map((point) => new THREE.Vector2(point[0], point[1]));
  let raw;
  try {
    raw = THREE.ShapeUtils.triangulateShape(contour, []);
  } catch {
    raw = [];
  }
  const out = [];
  for (const triangle of raw) {
    if (triangle.length !== 3) continue;
    if (triangle.some((index) => index < 0 || index >= points.length)) continue;
    out.push(orient(points, triangle[0], triangle[1], triangle[2]));
  }
  if (out.length === 0) {
    for (let index = 1; index + 1 < points.length; index += 1) {
      out.push(orient(points, 0, index, index + 1));
    }
  }
  return out;
});
process.stdout.write(JSON.stringify(result));
"#;

    let positions = cp
        .vertices
        .iter()
        .map(|item| (item.id, item.pos))
        .collect::<HashMap<_, _>>();
    let polygons = faces
        .iter()
        .map(|face| {
            face.vertices
                .iter()
                .map(|vertex_id| positions[vertex_id])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let input = serde_json::to_vec(&polygons).expect("serialize face polygons for Three.js");
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut child = Command::new("node")
        .args(["--input-type=module", "-e", SCRIPT])
        .current_dir(desktop)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Node with the checked-in Three.js dependency");
    child
        .stdin
        .take()
        .expect("Node stdin is piped")
        .write_all(&input)
        .expect("send face polygons to Node");
    let output = child
        .wait_with_output()
        .expect("wait for Node triangulation");
    assert!(
        output.status.success(),
        "Three.js triangulation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let by_face: Vec<Vec<[usize; 3]>> =
        serde_json::from_slice(&output.stdout).expect("read Three.js triangle indices");
    assert_eq!(by_face.len(), faces.len());
    faces
        .iter()
        .zip(by_face)
        .map(|(face, triangles)| (face.id, triangles))
        .collect()
}

fn camera(width: f64, height: f64, sign: f64) -> Camera {
    let center = V3::new(width * 0.5, height * 0.5, 0.0);
    let direction = V3::new(0.35, -0.85, 0.95).normalize() * sign;
    let tan_half_fov = (0.5 * CAMERA_FOV_DEG.to_radians()).tan();
    let distance = width.max(height) / (2.0 * tan_half_fov) * CAMERA_MARGIN;
    let position = center + direction * distance;
    let forward = (center - position).normalize();
    let right = forward.cross(V3::Y).normalize();
    let up = right.cross(forward).normalize();
    let view_row = |axis: V3| {
        [
            axis.x as f32,
            axis.y as f32,
            axis.z as f32,
            (-axis.dot(position)) as f32,
        ]
    };
    Camera {
        position,
        view_x: view_row(right),
        view_y: view_row(up),
        view_depth: view_row(forward),
        projection_scale: (1.0 / tan_half_fov) as f32,
        projection_depth_a: ((CAMERA_FAR + CAMERA_NEAR) / (CAMERA_FAR - CAMERA_NEAR)) as f32,
        projection_depth_b: (-2.0 * CAMERA_FAR * CAMERA_NEAR / (CAMERA_FAR - CAMERA_NEAR)) as f32,
    }
}

fn dot4(row: [f32; 4], point: [f32; 4]) -> f32 {
    row[0] * point[0] + row[1] * point[1] + row[2] * point[2] + row[3] * point[3]
}

fn project(camera: Camera, point: V3, viewport: usize) -> Option<Projected> {
    let point = [point.x as f32, point.y as f32, point.z as f32, 1.0];
    let view_depth = dot4(camera.view_depth, point);
    if !view_depth.is_finite() || view_depth <= CAMERA_NEAR as f32 {
        return None;
    }
    let ndc_x = dot4(camera.view_x, point) * camera.projection_scale / view_depth;
    let ndc_y = dot4(camera.view_y, point) * camera.projection_scale / view_depth;
    let clip_z = camera.projection_depth_a * view_depth + camera.projection_depth_b;
    let depth = (clip_z / view_depth) * 0.5 + 0.5;
    Some(Projected {
        x: (ndc_x + 1.0) * 0.5 * viewport as f32,
        y: (1.0 - ndc_y) * 0.5 * viewport as f32,
        depth,
    })
}

fn canonicalize(normal: &mut V3) -> bool {
    let ax = normal.x.abs();
    let ay = normal.y.abs();
    let az = normal.z.abs();
    let component = if ax >= ay && ax >= az {
        normal.x
    } else if ay >= az {
        normal.y
    } else {
        normal.z
    };
    if component < 0.0 {
        *normal = -*normal;
        true
    } else {
        false
    }
}

fn render_faces(
    diagram: &Diagram,
    frame: &Frame3D,
    viewport: usize,
    view: Camera,
) -> Vec<RenderFace> {
    frame
        .faces
        .iter()
        .map(|face| {
            let polygon = face
                .polygon
                .iter()
                .map(|point| {
                    V3::new(
                        point[0] as f32 as f64,
                        point[1] as f32 as f64,
                        point[2] as f32 as f64,
                    )
                })
                .collect::<Vec<_>>();
            let mut order_normal = V3::ZERO;
            let mut order_center = V3::ZERO;
            let mut order_points = 0_usize;
            for indices in &diagram.triangles[&face.face] {
                let a = polygon[indices[0]];
                let b = polygon[indices[1]];
                let c = polygon[indices[2]];
                order_normal += (b - a).cross(c - a);
                order_center += a + b + c;
                order_points += 3;
            }
            if order_points > 0 {
                order_center /= order_points as f64;
            }
            if order_normal.length_squared() <= f64::EPSILON {
                order_normal = V3::Z;
            } else {
                order_normal = order_normal.normalize();
            }
            let material_orientation = if canonicalize(&mut order_normal) {
                -1
            } else {
                1
            };
            let side = if order_normal.dot(view.position - order_center) >= 0.0 {
                1
            } else {
                -1
            };
            let triangles = diagram.triangles[&face.face]
                .iter()
                .filter_map(|indices| {
                    let points = [
                        polygon[indices[0]],
                        polygon[indices[1]],
                        polygon[indices[2]],
                    ];
                    let normal = (points[1] - points[0]).cross(points[2] - points[0]);
                    let center = (points[0] + points[1] + points[2]) / 3.0;
                    Some(RenderTriangle {
                        projected: [
                            project(view, points[0], viewport)?,
                            project(view, points[1], viewport)?,
                            project(view, points[2], viewport)?,
                        ],
                        back_facing: normal.dot(view.position - center) < 0.0,
                    })
                })
                .collect();
            RenderFace {
                face: face.face,
                owner_code: diagram.owner_codes[&face.face],
                surface_rank: face.surface_rank,
                side,
                material_orientation,
                triangles,
            }
        })
        .collect()
}

fn raster_bounds(triangle: &[Projected; 3], viewport: usize) -> (usize, usize, usize, usize) {
    let min_x = triangle
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let max_x = triangle
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(viewport as f32 - 1.0)
        .max(0.0) as usize;
    let min_y = triangle
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let max_y = triangle
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(viewport as f32 - 1.0)
        .max(0.0) as usize;
    (min_x, max_x, min_y, max_y)
}

fn rasterize(mut visit: impl FnMut(usize, f32), triangle: &[Projected; 3], viewport: usize) {
    let [a, b, c] = *triangle;
    let denominator = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if denominator.abs() <= RASTER_EPS {
        return;
    }
    let (min_x, max_x, min_y, max_y) = raster_bounds(triangle, viewport);
    if min_x > max_x || min_y > max_y {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let wa = ((b.y - c.y) * (px - c.x) + (c.x - b.x) * (py - c.y)) / denominator;
            let wb = ((c.y - a.y) * (px - c.x) + (a.x - c.x) * (py - c.y)) / denominator;
            let wc = 1.0 - wa - wb;
            if wa >= -RASTER_EPS && wb >= -RASTER_EPS && wc >= -RASTER_EPS {
                let depth = wa * a.depth + wb * b.depth + wc * c.depth;
                if (0.0..=1.0).contains(&depth) {
                    visit(y * viewport + x, depth);
                }
            }
        }
    }
}

fn visual_image(diagram: &Diagram, frame: &Frame3D, viewport: usize, view: Camera) -> VisualImage {
    let mut faces = render_faces(diagram, frame, viewport, view);
    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    let mut nearest = vec![u32::MAX; viewport * viewport];
    for face in &faces {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let code = (depth.clamp(0.0, 1.0) * max_depth_code as f32).round() as u32;
                    nearest[pixel] = nearest[pixel].min(code);
                },
                &triangle.projected,
                viewport,
            );
        }
    }

    faces.sort_by(|left, right| {
        (left.side * i64::from(left.surface_rank))
            .cmp(&(right.side * i64::from(right.surface_rank)))
            .then(left.surface_rank.cmp(&right.surface_rank))
            .then(left.material_orientation.cmp(&right.material_orientation))
            .then((left.side * i64::from(left.face)).cmp(&(right.side * i64::from(right.face))))
            .then((left.side * left.owner_code).cmp(&(right.side * right.owner_code)))
    });
    let tolerance = DEPTH_TIE_CODES as f32 / max_depth_code as f32;
    let mut owners = vec![None::<(usize, bool)>; viewport * viewport];
    for (face_index, face) in faces.iter().enumerate() {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let nearest_code = nearest[pixel];
                    if nearest_code == u32::MAX {
                        return;
                    }
                    let nearest_depth = nearest_code as f32 / max_depth_code as f32;
                    if depth - nearest_depth <= tolerance {
                        owners[pixel] = Some((face_index, triangle.back_facing));
                    }
                },
                &triangle.projected,
                viewport,
            );
        }
    }

    let mut visible_faces = BTreeSet::new();
    let mut visible_back_faces = BTreeSet::new();
    let mut red_pixels = 0;
    let mut light_pixels = 0;
    for (owner, back_facing) in owners.into_iter().flatten() {
        let face = &faces[owner];
        visible_faces.insert(face.face);
        if back_facing {
            visible_back_faces.insert(face.face);
            light_pixels += 1;
        } else {
            red_pixels += 1;
        }
    }
    VisualImage {
        visible_faces,
        visible_back_faces,
        red_pixels,
        light_pixels,
    }
}

fn endpoint_frames(diagram: &Diagram, hinge: EdgeId, sign: f64) -> (Frame3D, Frame3D) {
    let mut warm = None::<HashMap<EdgeId, f64>>;
    for absolute in WARMUP_ABS {
        let motion = solve_motion(
            &diagram.cp,
            &diagram.faces,
            &[Driver {
                hinge,
                target_angle_deg: sign * absolute,
            }],
            None,
            warm.as_ref(),
            true,
        );
        warm = Some(motion.result.angles);
    }

    let mut before = None;
    let mut after = None;
    for absolute in BOUNDARY_ABS {
        let motion = solve_motion(
            &diagram.cp,
            &diagram.faces,
            &[Driver {
                hinge,
                target_angle_deg: sign * absolute,
            }],
            None,
            warm.as_ref(),
            true,
        );
        assert!(
            motion
                .result
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite()),
            "{} edge {hinge} at {} degrees returned non-finite geometry",
            diagram.name,
            sign * absolute
        );
        if absolute == 179.999 {
            before = Some(motion.result.frame.clone());
        } else if absolute == 180.0 {
            after = Some(motion.result.frame.clone());
        }
        warm = Some(motion.result.angles);
    }
    (
        before.expect("boundary samples include 179.999 degrees"),
        after.expect("boundary samples include 180 degrees"),
    )
}

fn ids(values: &BTreeSet<FaceId>) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn surface_rank_order(frame: &Frame3D) -> Vec<FaceId> {
    let mut ranks = frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    ranks.sort_unstable();
    ranks.into_iter().map(|(_, face)| face).collect()
}

#[test]
#[ignore = "full 110-crease acceptance raster sweep; run explicitly in release mode"]
fn surface_order_179_999_to_180_all_110_creases() {
    let diagrams = boundary_diagrams();
    let mut total_hinges = 0;
    let mut changed_hinges = BTreeSet::<(&'static str, EdgeId)>::new();
    let mut changed_directions = 0;
    let mut rank_changed_hinges = BTreeSet::<(&'static str, EdgeId)>::new();
    let mut rank_changed_directions = 0;
    for diagram in &diagrams {
        let mut diagram_changed = BTreeSet::new();
        let mut diagram_rank_changed = BTreeSet::new();
        total_hinges += diagram.hinges.len();
        for &(hinge, kind) in &diagram.hinges {
            for sign in [1.0, -1.0] {
                let (before_frame, after_frame) = endpoint_frames(diagram, hinge, sign);
                if surface_rank_order(&before_frame) != surface_rank_order(&after_frame) {
                    rank_changed_hinges.insert((diagram.name, hinge));
                    rank_changed_directions += 1;
                    diagram_rank_changed.insert(hinge);
                }
                let view = camera(diagram.paper_width, diagram.paper_height, 1.0);
                let before = visual_image(diagram, &before_frame, VIEWPORT, view);
                let after = visual_image(diagram, &after_frame, VIEWPORT, view);
                if before.visible_back_faces != after.visible_back_faces {
                    changed_directions += 1;
                    changed_hinges.insert((diagram.name, hinge));
                    diagram_changed.insert(hinge);
                    println!(
                        "SURFACE_180_CHANGE diagram={} edge={} kind={kind:?} direction={sign:+} before_back={} after_back={}",
                        diagram.name,
                        hinge,
                        ids(&before.visible_back_faces),
                        ids(&after.visible_back_faces),
                    );
                }
            }
        }
        println!(
            "SURFACE_180_DIAGRAM diagram={} hinges={} changed_hinges={} changed_ids={}",
            diagram.name,
            diagram.hinges.len(),
            diagram_changed.len(),
            ids(&diagram_changed),
        );
        println!(
            "SURFACE_180_RANK_DIAGRAM diagram={} hinges={} changed_hinges={} changed_ids={}",
            diagram.name,
            diagram.hinges.len(),
            diagram_rank_changed.len(),
            ids(&diagram_rank_changed),
        );
    }
    println!(
        "SURFACE_180_TOTAL diagrams={} hinges={} directions={} changed_hinges={} changed_directions={}",
        diagrams.len(),
        total_hinges,
        total_hinges * 2,
        changed_hinges.len(),
        changed_directions,
    );
    println!(
        "SURFACE_180_RANK_TOTAL changed_hinges={} changed_directions={} changed_edges={rank_changed_hinges:?}",
        rank_changed_hinges.len(),
        rank_changed_directions,
    );
    assert_eq!(total_hinges, 110);
}

#[test]
fn surface_order_minus_94_four_view_conditions() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    assert_eq!(cp.vertices.len(), 14);
    assert_eq!(cp.edges.len(), 29);
    assert_eq!(faces.len(), 16);
    let diagram = diagram("diagonal-midline-user-cp", cp.clone(), 1.0, 1.0);
    let mut valley_front = None;
    for (fold, angle) in [("mountain", 94.0), ("valley", -94.0)] {
        let folded = propagate(&cp, &faces, &angle_surface_angles(angle));
        let frame = to_frame3d(&cp, &faces, &folded);
        for (view_name, sign) in [("front", 1.0), ("back", -1.0)] {
            let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, sign));
            println!(
                "SURFACE_94 fold={fold} angle={angle:+.0} view={view_name} red={} light={} red_ratio={:.6}% visible={} back_faces={}",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                ids(&image.visible_faces),
                ids(&image.visible_back_faces),
            );
            assert!(image.red_pixels + image.light_pixels > 0);
            if fold == "valley" && view_name == "front" {
                valley_front = Some(image);
            }
        }
    }
    let valley_front = valley_front.expect("the four conditions include valley/front");
    assert!(
        valley_front.red_ratio() > 0.5,
        "valley -94 degrees from the front must be majority red: red={} light={}",
        valley_front.red_pixels,
        valley_front.light_pixels,
    );
}

#[test]
fn surface_order_same_input_is_deterministic_ten_times() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("diagonal-midline-user-cp", cp.clone(), 1.0, 1.0);
    let angles = angle_surface_angles(-94.0);
    let mut expected = None::<(Vec<u8>, VisualImage)>;
    for run in 1..=10 {
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let serialized = serde_json::to_vec(&frame).expect("serialize deterministic frame");
        let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, 1.0));
        if let Some(reference) = &expected {
            assert_eq!(&serialized, &reference.0, "frame changed on run {run}");
            assert_eq!(&image, &reference.1, "visible result changed on run {run}");
        } else {
            expected = Some((serialized, image));
        }
    }
    println!("SURFACE_DETERMINISM identical_runs=10 input=edge43:-94");
}
