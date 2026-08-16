//! Browserless acceptance measurements for the rigid surface-order contract.
//!
//! The raster path intentionally follows Viewer3D's Float32 projection,
//! 24-bit depth target, two-code depth tolerance, and surface-owner ordering.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, Neg, Sub};

use ori3_cp::{Face, extract_faces};
use ori3_model::{
    CreasePattern, Document, Driver, Edge, EdgeId, EdgeKind, FaceId, Frame3D, Vertex,
};
use ori3_rigid::{SurfaceOrderSource, propagate, solve_motion, to_frame3d};

const CAMERA_FOV_DEG: f64 = 45.0;
const CAMERA_MARGIN: f64 = 1.35;
const CAMERA_NEAR: f64 = 0.01;
const CAMERA_FAR: f64 = 100.0;
const DEPTH_BITS: u32 = 24;
const DEPTH_TIE_CODES: u32 = 2;
const VIEWPORT: usize = 800;
const RASTER_EPS: f32 = 1e-10;
const SURFACE_OWNER_PLANARITY_EPSILON: f64 = 1e-6;
const SURFACE_OWNER_COPLANAR_EPSILON: f64 = 1e-6;
const SURFACE_OWNER_NORMAL_EPSILON: f64 = 1e-6;

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
    pixels: Vec<Option<VisualPixel>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisualPixel {
    face: FaceId,
    back_facing: bool,
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

    fn difference(&self, other: &Self, viewport: usize) -> VisualDifference {
        assert_eq!(self.pixels.len(), other.pixels.len());
        assert_eq!(self.pixels.len(), viewport * viewport);
        let mut owner_pixels = 0;
        let mut color_pixels = 0;
        let mut coverage_pixels = 0;
        let mut side_pixels = 0;
        let mut face_only_pixels = 0;
        let mut min_x = viewport;
        let mut min_y = viewport;
        let mut max_x = 0;
        let mut max_y = 0;
        for (pixel, (before, after)) in self.pixels.iter().zip(&other.pixels).enumerate() {
            if before != after {
                owner_pixels += 1;
            }
            let color_changed = match (before, after) {
                (None, None) => false,
                (None, Some(_)) | (Some(_), None) => {
                    coverage_pixels += 1;
                    true
                }
                (Some(before), Some(after)) if before.back_facing != after.back_facing => {
                    side_pixels += 1;
                    true
                }
                (Some(before), Some(after)) => {
                    if before.face != after.face {
                        face_only_pixels += 1;
                    }
                    false
                }
            };
            if color_changed {
                color_pixels += 1;
                let x = pixel % viewport;
                let y = pixel / viewport;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        VisualDifference {
            owner_pixels,
            color_pixels,
            coverage_pixels,
            side_pixels,
            face_only_pixels,
            color_bounds: (color_pixels > 0).then_some((min_x, min_y, max_x, max_y)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisualDifference {
    owner_pixels: usize,
    color_pixels: usize,
    coverage_pixels: usize,
    side_pixels: usize,
    face_only_pixels: usize,
    color_bounds: Option<(usize, usize, usize, usize)>,
}

#[derive(Clone)]
struct EndpointState {
    frame: Frame3D,
    angles: HashMap<EdgeId, f64>,
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
    plane_normal: V3,
    plane_distance: f64,
    planar: bool,
    /// 支持平面ごとに付ける組の符号。0は平面をまとめられなかったこと。
    /// Viewer3Dの `surfaceOwnerGroupToken` と同じ規則で付ける。
    coplanar_group: u32,
    points: Vec<V3>,
    triangles: Vec<RenderTriangle>,
}

struct RenderTriangle {
    projected: [Projected; 3],
    depth_plane: Option<[f32; 3]>,
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

/// `verification/angles.json` の展開図を丸めず、追跡対象の検査コード内へ写したもの。
fn zero_back_user_cp() -> CreasePattern {
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
            vertex(11, 0.207_106_781_186_547_52, 0.5),
            vertex(12, 0.5, 0.792_893_218_813_452_5),
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
            edge(27, 4, 11, EdgeKind::Mountain),
            edge(28, 11, 8, EdgeKind::Mountain),
            edge(29, 0, 11, EdgeKind::Mountain),
            edge(30, 11, 3, EdgeKind::Mountain),
            edge(31, 6, 12, EdgeKind::Mountain),
            edge(32, 12, 8, EdgeKind::Mountain),
            edge(33, 2, 12, EdgeKind::Mountain),
            edge(34, 12, 3, EdgeKind::Mountain),
            edge(35, 11, 12, EdgeKind::Valley),
            edge(36, 9, 10, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// `verification/angles.json` の20角度。JSONのf64値をそのまま使い、丸めない。
fn zero_back_user_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -179.999_999_999_999_97),
        (18, -179.999_999_999_999_97),
        (19, 180.0),
        (20, -92.999_999_999_999_99),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -92.999_999_999_999_99),
        (25, 180.0),
        (26, 180.0),
        (27, -178.693_545_755_435_96),
        (28, 180.0),
        (29, 180.0),
        (30, 180.0),
        (31, -178.693_545_755_435_96),
        (32, 179.999_999_999_999_97),
        (33, 180.0),
        (34, 180.0),
        (35, -1.306_454_244_564_035_5),
        (36, -87.0),
    ])
}

/// `verification/angles-now.json` の20角度。JSONのf64値をそのまま使い、丸めない。
/// `zero_back_user_angles` との違いは辺20/24/27/28/31/32/35/36の8本。
fn zero_back_user_angles_now() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -179.999_999_999_999_97),
        (18, -179.999_999_999_999_97),
        (19, 180.0),
        (20, -97.999_999_999_999_97),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -98.000_000_000_000_01),
        (25, 180.0),
        (26, 180.0),
        (27, -179.128_990_221_440_23),
        (28, 179.999_999_999_999_97),
        (29, 180.0),
        (30, 180.0),
        (31, -179.128_990_221_440_23),
        (32, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -0.871_009_778_559_739_8),
        (36, -82.0),
    ])
}

/// 手順0件のFrameへ、既定オンの重なり補正をproductionと同じ順で適用する。
/// 動作中の `desktop.exe` から読み取った展開図(頂点13・辺28)。
/// `zero_back_user_cp` と同じ正方形だが、辺と頂点の番号は実機のものをそのまま使う。
/// `.gitignore` 対象の `verification/` を検査から読まないため値を埋め込んでいる
/// (CLAUDE.md §10.1)。
fn live_frame_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 1.0, 0.5),
            vertex(5, 0.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.5, 0.792_893_218_813_452_5),
            vertex(10, 0.792_893_218_813_452_5, 0.5),
            vertex(11, 0.5, 0.207_106_781_186_547_52),
            vertex(12, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 1, 4, EdgeKind::Border),
            edge(5, 4, 2, EdgeKind::Border),
            edge(6, 3, 5, EdgeKind::Border),
            edge(7, 5, 0, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 6, 9, EdgeKind::Mountain),
            edge(20, 9, 8, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 4, 10, EdgeKind::Mountain),
            edge(23, 10, 8, EdgeKind::Mountain),
            edge(24, 2, 10, EdgeKind::Mountain),
            edge(25, 10, 1, EdgeKind::Mountain),
            edge(26, 8, 11, EdgeKind::Mountain),
            edge(27, 11, 7, EdgeKind::Mountain),
            edge(28, 0, 11, EdgeKind::Mountain),
            edge(29, 11, 1, EdgeKind::Mountain),
            edge(30, 8, 12, EdgeKind::Mountain),
            edge(31, 12, 5, EdgeKind::Mountain),
            edge(32, 0, 12, EdgeKind::Mountain),
            edge(33, 12, 3, EdgeKind::Mountain),
            edge(34, 3, 9, EdgeKind::Mountain),
            edge(35, 12, 9, EdgeKind::Valley),
            edge(36, 10, 11, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// 実機の `poseAngles` 20本をそのまま写した。f64の値は丸めない。
/// このうち ±180°(誤差 `1e-6` 以内)は15本ある。
fn live_frame_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (19, -178.265_130_385_534_97),
        (20, 180.0),
        (21, 180.0),
        (22, -3.062_204_584_590_538_5e-15),
        (23, 180.0),
        (24, 180.0),
        (25, 180.0),
        (26, 180.0),
        (27, -5.233_885_113_024_099e-15),
        (28, 180.0),
        (29, 180.0),
        (30, 179.999_999_999_999_97),
        (31, -178.265_130_385_534_97),
        (32, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -1.734_869_614_465_027),
        (36, -180.0),
    ])
}

/// 実機が表示していた形を、productionと同じ順で組み立てる。
fn live_frame_frame(cp: &CreasePattern, faces: &[Face]) -> Frame3D {
    let folded = propagate(cp, faces, &live_frame_angles());
    zero_back_apply_overlap(cp, faces, to_frame3d(cp, faces, &folded))
}

fn zero_back_apply_overlap(cp: &CreasePattern, faces: &[Face], mut frame: Frame3D) -> Frame3D {
    let order = crate::store::frame_surface_rank_order(&frame)
        .expect("the zero-back frame has a complete unique surface order");
    ori3_soft::prevent_overlap_with_order_authority(
        cp,
        faces,
        &mut frame,
        ori3_soft::OverlapOrderInput {
            start: &order,
            end: &order,
            progress: 0.5,
            authoritative: true,
        },
        &ori3_soft::OverlapSettings {
            enabled: true,
            ..Default::default()
        },
    );
    frame
}

/// 手順0件で保持されている全ヒンジ角から表示Frameを再構成し、既定オンの
/// 重なり補正までproductionと同じ順で適用する。
fn zero_back_user_frame(cp: &CreasePattern, faces: &[Face]) -> Frame3D {
    let folded = propagate(cp, faces, &zero_back_user_angles());
    zero_back_apply_overlap(cp, faces, to_frame3d(cp, faces, &folded))
}

#[derive(Clone, Copy, Debug)]
enum ZeroBackWarmPath {
    /// 選択中の辺36をhard、残る19角をpreferredとして同時に最終値へ近づける。
    Active36WithPreferred,
    /// 利用者の操作仮説どおり、辺36だけをhardにして残る19本を自由追従させる。
    Edge36Only,
    /// 全20角をhardにする対照。自由変数が無いためwarm startで枝を選べない。
    AllHardControl,
}

impl ZeroBackWarmPath {
    fn label(self) -> &'static str {
        match self {
            Self::Active36WithPreferred => "edge36-hard-plus-19-preferred",
            Self::Edge36Only => "edge36-hard-only",
            Self::AllHardControl => "all-20-hard-control",
        }
    }
}

#[derive(Debug)]
struct ZeroBackWarmResult {
    frame: Frame3D,
    angles: HashMap<EdgeId, f64>,
    converged: bool,
    closure_rms: f64,
    iterations: u32,
    contact_detected: bool,
    self_intersects: bool,
    max_seam_gap: f64,
}

fn zero_back_warm_solve(
    cp: &CreasePattern,
    faces: &[Face],
    path: ZeroBackWarmPath,
    stages: usize,
) -> ZeroBackWarmResult {
    zero_back_warm_solve_to(cp, faces, &zero_back_user_angles(), path, stages)
}

fn zero_back_warm_solve_to(
    cp: &CreasePattern,
    faces: &[Face],
    final_angles: &HashMap<EdgeId, f64>,
    path: ZeroBackWarmPath,
    stages: usize,
) -> ZeroBackWarmResult {
    assert!(stages > 0);
    let final_angles = final_angles.clone();
    let mut warm = final_angles
        .keys()
        .copied()
        .map(|hinge| (hinge, 0.0))
        .collect::<HashMap<_, _>>();
    let mut last = None;

    for stage in 1..=stages {
        let progress = stage as f64 / stages as f64;
        let scaled = |hinge: EdgeId| final_angles[&hinge] * progress;
        let (hard, preferred) = match path {
            ZeroBackWarmPath::Active36WithPreferred => (
                vec![Driver {
                    hinge: 36,
                    target_angle_deg: scaled(36),
                }],
                Some(
                    final_angles
                        .keys()
                        .copied()
                        .filter(|&hinge| hinge != 36)
                        .map(|hinge| (hinge, scaled(hinge)))
                        .collect::<HashMap<_, _>>(),
                ),
            ),
            ZeroBackWarmPath::Edge36Only => (
                vec![Driver {
                    hinge: 36,
                    target_angle_deg: scaled(36),
                }],
                None,
            ),
            ZeroBackWarmPath::AllHardControl => {
                let mut hard = final_angles
                    .keys()
                    .copied()
                    .map(|hinge| Driver {
                        hinge,
                        target_angle_deg: scaled(hinge),
                    })
                    .collect::<Vec<_>>();
                hard.sort_unstable_by_key(|driver| driver.hinge);
                (hard, None)
            }
        };
        let motion = solve_motion(cp, faces, &hard, preferred.as_ref(), Some(&warm), true);
        assert!(
            motion
                .result
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite()),
            "{} with {stages} external stages returned non-finite geometry at stage {stage}",
            path.label(),
        );
        assert_eq!(motion.result.frame.faces.len(), faces.len());
        assert_eq!(motion.result.angles.len(), final_angles.len());
        warm = motion.result.angles.clone();
        last = Some(motion);
    }

    let motion = last.expect("at least one external warm-start stage ran");
    let self_intersects = ori3_rigid::self_intersects(&motion.result.frame);
    let max_seam_gap = ori3_rigid::max_seam_gap(cp, faces, &motion.result.frame);
    ZeroBackWarmResult {
        frame: zero_back_apply_overlap(cp, faces, motion.result.frame),
        angles: motion.result.angles,
        converged: motion.result.converged,
        closure_rms: motion.result.closure_rms,
        iterations: motion.result.iterations,
        contact_detected: motion.contact_detected,
        self_intersects,
        max_seam_gap,
    }
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
    let positions = cp
        .vertices
        .iter()
        .map(|item| (item.id, item.pos))
        .collect::<HashMap<_, _>>();
    faces
        .iter()
        .map(|face| {
            let polygon = face
                .vertices
                .iter()
                .map(|vertex_id| positions[vertex_id])
                .collect::<Vec<_>>();
            (face.id, triangulate_polygon(&polygon))
        })
        .collect()
}

/// Viewer3Dと同じく、反時計回りの単純多角形を耳切り法で三角形へ分ける。
/// スリット等の退化で耳を切れなくなった場合も検査を止めず、残りを扇形に分ける。
fn triangulate_polygon(points: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let mut triangles = Vec::with_capacity(points.len().saturating_sub(2));
    if points.len() < 3 {
        return triangles;
    }

    let cross = |a: usize, b: usize, c: usize| {
        (points[b][0] - points[a][0]) * (points[c][1] - points[a][1])
            - (points[b][1] - points[a][1]) * (points[c][0] - points[a][0])
    };
    let orient = |a: usize, b: usize, c: usize| {
        if cross(a, b, c) < 0.0 {
            [a, c, b]
        } else {
            [a, b, c]
        }
    };
    let mut remaining = (0..points.len()).collect::<Vec<_>>();
    while remaining.len() > 3 {
        let count = remaining.len();
        let mut ear = None;
        for index in 0..count {
            let a = remaining[(index + count - 1) % count];
            let b = remaining[index];
            let c = remaining[(index + 1) % count];
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            let blocked = remaining.iter().any(|&point| {
                point != a
                    && point != b
                    && point != c
                    && cross(a, b, point) >= 0.0
                    && cross(b, c, point) >= 0.0
                    && cross(c, a, point) >= 0.0
            });
            if !blocked {
                ear = Some(index);
                break;
            }
        }
        let Some(index) = ear else {
            break;
        };
        let count = remaining.len();
        triangles.push(orient(
            remaining[(index + count - 1) % count],
            remaining[index],
            remaining[(index + 1) % count],
        ));
        remaining.remove(index);
    }
    for index in 1..remaining.len().saturating_sub(1) {
        triangles.push(orient(remaining[0], remaining[index], remaining[index + 1]));
    }
    triangles
}

fn camera_from_direction(width: f64, height: f64, direction: V3) -> Camera {
    let center = V3::new(width * 0.5, height * 0.5, 0.0);
    let direction = direction.normalize();
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

fn camera(width: f64, height: f64, sign: f64) -> Camera {
    camera_from_direction(width, height, V3::new(0.35, -0.85, 0.95).normalize() * sign)
}

/// Viewer3Dの横長canvasと同じ水平画角にする。既存のCPU rasterはaspect=1固定なので、
/// view行のx側だけを割り、clipping範囲を画面と同じにして測定する。
fn camera_with_aspect(width: f64, height: f64, sign: f64, aspect: f64) -> Camera {
    assert!(aspect.is_finite() && aspect > 0.0);
    let mut result = camera(width, height, sign);
    for value in &mut result.view_x {
        *value /= aspect as f32;
    }
    result
}

/// Viewer3DのY-up軌道カメラと同じ球面方向を、方位角と仰角から作る。
/// 方位角0度は紙の表法線(+Z)、90度は+X、仰角はY方向を正とする。
fn camera_from_orbit_angles(
    width: f64,
    height: f64,
    azimuth_deg: i32,
    elevation_deg: i32,
) -> Camera {
    let azimuth = f64::from(azimuth_deg).to_radians();
    let elevation = f64::from(elevation_deg).to_radians();
    let horizontal = elevation.cos();
    camera_from_direction(
        width,
        height,
        V3::new(
            horizontal * azimuth.sin(),
            elevation.sin(),
            horizontal * azimuth.cos(),
        ),
    )
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

fn camera_plane_depth(camera: Camera, normal: V3, distance: f64) -> Option<[f32; 3]> {
    let row_vector =
        |row: [f32; 4]| V3::new(f64::from(row[0]), f64::from(row[1]), f64::from(row[2]));
    let view_x = row_vector(camera.view_x);
    let view_y = row_vector(camera.view_y);
    let view_depth = row_vector(camera.view_depth);
    let determinant = view_x.dot(view_y.cross(view_depth));
    if !determinant.is_finite() || determinant.abs() <= SURFACE_OWNER_NORMAL_EPSILON {
        return None;
    }
    // Columns of the inverse view linear transform.  Unlike assuming an
    // orthonormal basis, this also covers camera_with_aspect's scaled x row.
    let view_x_dual = view_y.cross(view_depth) / determinant;
    let view_y_dual = view_depth.cross(view_x) / determinant;
    let view_depth_dual = view_x.cross(view_y) / determinant;
    let projection_scale = f64::from(camera.projection_scale);
    let camera_plane_distance = normal.dot(camera.position) - distance;
    if !projection_scale.is_finite()
        || projection_scale.abs() <= f64::EPSILON
        || camera_plane_distance.abs() <= SURFACE_OWNER_COPLANAR_EPSILON
    {
        return None;
    }

    // CPU equivalent of surfaceOwnerCanonicalDepth in surfaceOwnerShader.ts.
    let depth_scale = -0.5 * f64::from(camera.projection_depth_b) / camera_plane_distance;
    let x = depth_scale * normal.dot(view_x_dual) / projection_scale;
    let y = depth_scale * normal.dot(view_y_dual) / projection_scale;
    let constant = 0.5 * (f64::from(camera.projection_depth_a) + 1.0)
        + depth_scale * normal.dot(view_depth_dual);
    let coefficients = [x as f32, y as f32, constant as f32];
    coefficients
        .iter()
        .all(|value| value.is_finite())
        .then_some(coefficients)
}

fn render_face_owner_key(face: &RenderFace) -> (i64, u32, i64, i64, i64) {
    (
        face.side * i64::from(face.surface_rank),
        face.surface_rank,
        face.material_orientation,
        face.side * i64::from(face.face),
        face.side * face.owner_code,
    )
}

fn render_faces(
    diagram: &Diagram,
    frame: &Frame3D,
    viewport: usize,
    view: Camera,
) -> Vec<RenderFace> {
    let mut rendered = frame
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
            let normal_valid = order_normal.length_squared()
                > SURFACE_OWNER_NORMAL_EPSILON * SURFACE_OWNER_NORMAL_EPSILON;
            if !normal_valid {
                order_normal = V3::Z;
            } else {
                order_normal = order_normal.normalize();
            }
            // Match updateBatchViewOrder: material orientation uses the raw
            // normalized winding, before canonicalizing the plane normal.
            let material_orientation = if order_normal.z < 0.0 { -1 } else { 1 };
            canonicalize(&mut order_normal);
            let side = if order_normal.dot(view.position - order_center) >= 0.0 {
                1
            } else {
                -1
            };
            let plane_normal = order_normal;
            let plane_distance = plane_normal.dot(order_center);
            let planar = normal_valid
                && polygon.iter().all(|point| {
                    (plane_normal.dot(*point) - plane_distance).abs()
                        <= SURFACE_OWNER_PLANARITY_EPSILON
                });
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
                        depth_plane: None,
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
                plane_normal,
                plane_distance,
                planar,
                coplanar_group: 0,
                points: polygon,
                triangles,
            }
        })
        .collect::<Vec<_>>();

    // Match Viewer3D's common supporting-plane rule. Stable face-id traversal
    // plus a fixed group anchor prevents tolerance chaining from merging
    // distinct physical layers.
    let mut face_indices = (0..rendered.len()).collect::<Vec<_>>();
    face_indices.sort_unstable_by_key(|&index| rendered[index].face);
    let mut strict_groups = Vec::<Vec<usize>>::new();
    for face_index in face_indices {
        if !rendered[face_index].planar {
            continue;
        }
        let matching_group = strict_groups.iter_mut().find(|members| {
            let anchor = &rendered[members[0]];
            let candidate = &rendered[face_index];
            let alignment = if anchor.plane_normal.dot(candidate.plane_normal) < 0.0 {
                -1.0
            } else {
                1.0
            };
            let candidate_normal = candidate.plane_normal * alignment;
            let candidate_distance = candidate.plane_distance * alignment;
            (candidate_normal - anchor.plane_normal)
                .length_squared()
                .sqrt()
                <= SURFACE_OWNER_NORMAL_EPSILON
                && (candidate_distance - anchor.plane_distance).abs()
                    <= SURFACE_OWNER_COPLANAR_EPSILON
                && candidate.points.iter().all(|point| {
                    (anchor.plane_normal.dot(*point) - anchor.plane_distance).abs()
                        <= SURFACE_OWNER_COPLANAR_EPSILON
                })
        });
        if let Some(members) = matching_group {
            members.push(face_index);
        } else {
            strict_groups.push(vec![face_index]);
        }
    }

    // Viewer3Dと同じく、支持平面ごとに組符号を配る。共平面の面が補間の丸めで
    // tie窓から外れても、重なり順で所有者を決められるようにするため。
    let mut coplanar_group = 0_u32;
    for members in &strict_groups {
        coplanar_group += 1;
        for &index in members {
            rendered[index].coplanar_group = coplanar_group;
        }
    }

    let maximum_depth_code = ((1_u64 << DEPTH_BITS) - 1) as f32;
    let residual_tolerance = DEPTH_TIE_CODES as f32 / maximum_depth_code;
    for members in strict_groups {
        let triangle_count = members
            .iter()
            .map(|&index| rendered[index].triangles.len())
            .sum::<usize>();
        if triangle_count <= 1 {
            continue;
        }
        let anchor = &rendered[members[0]];
        let Some(depth_plane) =
            camera_plane_depth(view, anchor.plane_normal, anchor.plane_distance)
        else {
            continue;
        };
        let residuals_valid = members.iter().all(|&index| {
            rendered[index].triangles.iter().all(|triangle| {
                triangle.projected.iter().all(|point| {
                    let ndc_x = point.x / viewport as f32 * 2.0 - 1.0;
                    let ndc_y = 1.0 - point.y / viewport as f32 * 2.0;
                    let shared_depth =
                        depth_plane[0] * ndc_x + depth_plane[1] * ndc_y + depth_plane[2];
                    (shared_depth - point.depth).abs() <= residual_tolerance
                })
            })
        });
        if !residuals_valid {
            continue;
        }
        for index in members {
            for triangle in &mut rendered[index].triangles {
                triangle.depth_plane = Some(depth_plane);
            }
        }
    }
    rendered
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

fn rasterize(mut visit: impl FnMut(usize, f32), triangle: &RenderTriangle, viewport: usize) {
    // Preserve the original vertex order for legacy coverage. Only strict
    // coplanar groups replace barycentric depth with their common NDC plane.
    let [a, b, c] = triangle.projected;
    let denominator = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if denominator.abs() <= RASTER_EPS {
        return;
    }
    let (min_x, max_x, min_y, max_y) = raster_bounds(&triangle.projected, viewport);
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
                let depth =
                    if let Some([x_coefficient, y_coefficient, constant]) = triangle.depth_plane {
                        let ndc_x = px / viewport as f32 * 2.0 - 1.0;
                        let ndc_y = 1.0 - py / viewport as f32 * 2.0;
                        x_coefficient * ndc_x + y_coefficient * ndc_y + constant
                    } else {
                        wa * a.depth + wb * b.depth + wc * c.depth
                    };
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
    // 第1passの描画順もGPUと同じ所有者順にそろえる。同じ最前深度を書いた面のうち
    // 最後に描いた面の組符号が残る、GLのLEQUAL + colorWriteと同じ規則にするため。
    faces.sort_by_key(render_face_owner_key);
    let mut nearest = vec![u32::MAX; viewport * viewport];
    let mut nearest_group = vec![0_u32; viewport * viewport];
    for face in &faces {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let code = (depth.clamp(0.0, 1.0) * max_depth_code as f32).round() as u32;
                    if code <= nearest[pixel] {
                        nearest[pixel] = code;
                        nearest_group[pixel] = face.coplanar_group;
                    }
                },
                triangle,
                viewport,
            );
        }
    }

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
                    // 最前面と同じ支持平面の面は、補間の丸めで深度がずれていても候補に残す。
                    let same_group = face.coplanar_group != 0
                        && face.coplanar_group == nearest_group[pixel];
                    let nearest_depth = nearest_code as f32 / max_depth_code as f32;
                    if same_group || depth - nearest_depth <= tolerance {
                        owners[pixel] = Some((face_index, triangle.back_facing));
                    }
                },
                triangle,
                viewport,
            );
        }
    }

    let mut visible_faces = BTreeSet::new();
    let mut visible_back_faces = BTreeSet::new();
    let mut red_pixels = 0;
    let mut light_pixels = 0;
    let pixels = owners
        .into_iter()
        .map(|owner| {
            owner.map(|(owner, back_facing)| {
                let face = &faces[owner];
                visible_faces.insert(face.face);
                if back_facing {
                    visible_back_faces.insert(face.face);
                    light_pixels += 1;
                } else {
                    red_pixels += 1;
                }
                VisualPixel {
                    face: face.face,
                    back_facing,
                }
            })
        })
        .collect();
    VisualImage {
        visible_faces,
        visible_back_faces,
        red_pixels,
        light_pixels,
        pixels,
    }
}

/// 利用者指定の相互排他的RGB条件で、fill-only rasterの表・裏画素を数える。
/// 背景・黒線・UI・輪郭AAはこのrasterへ入らない。
fn classified_fill_counts(image: &VisualImage) -> (u64, u64) {
    let mut front = 0_u64;
    let mut back = 0_u64;
    for pixel in image.pixels.iter().flatten() {
        let [r, g, b] = if pixel.back_facing {
            [255_i16, 255_i16, 255_i16]
        } else {
            [237_i16, 28_i16, 36_i16]
        };
        let is_front = r > 140 && r - g > 40 && r - b > 40;
        let is_background = (r - 205).abs() <= 12 && (g - 200).abs() <= 12 && (b - 193).abs() <= 12;
        let is_black = r < 90 && g < 90 && b < 90;
        let is_back = !is_front
            && !is_background
            && !is_black
            && r > 150
            && (r - g).abs() < 30
            && (g - b).abs() < 30;
        front += u64::from(is_front);
        back += u64::from(is_back);
    }
    (front, back)
}

fn endpoint_frames(diagram: &Diagram, hinge: EdgeId, sign: f64) -> (EndpointState, EndpointState) {
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
            before = Some(EndpointState {
                frame: motion.result.frame.clone(),
                angles: motion.result.angles.clone(),
            });
        } else if absolute == 180.0 {
            after = Some(EndpointState {
                frame: motion.result.frame.clone(),
                angles: motion.result.angles.clone(),
            });
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

fn max_vertex_distance(before: &Frame3D, after: &Frame3D) -> f64 {
    let after_faces = after
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<BTreeMap<_, _>>();
    before
        .faces
        .iter()
        .flat_map(|before_face| {
            let after_face = after_faces
                .get(&before_face.face)
                .expect("endpoint frames contain the same faces");
            assert_eq!(before_face.polygon.len(), after_face.polygon.len());
            before_face.polygon.iter().zip(&after_face.polygon).map(
                |(before_vertex, after_vertex)| {
                    before_vertex
                        .iter()
                        .zip(after_vertex)
                        .map(|(before_coordinate, after_coordinate)| {
                            (before_coordinate - after_coordinate).powi(2)
                        })
                        .sum::<f64>()
                        .sqrt()
                },
            )
        })
        .fold(0.0, f64::max)
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
                let (before_state, after_state) = endpoint_frames(diagram, hinge, sign);
                let before_order = surface_rank_order(&before_state.frame);
                let after_order = surface_rank_order(&after_state.frame);
                if before_order != after_order {
                    rank_changed_hinges.insert((diagram.name, hinge));
                    rank_changed_directions += 1;
                    diagram_rank_changed.insert(hinge);
                    println!(
                        "SURFACE_180_RANK_CHANGE diagram={} edge={} kind={kind:?} direction={sign:+} before_order={before_order:?} after_order={after_order:?}",
                        diagram.name, hinge,
                    );
                }
                let view = camera(diagram.paper_width, diagram.paper_height, 1.0);
                let before = visual_image(diagram, &before_state.frame, VIEWPORT, view);
                let after = visual_image(diagram, &after_state.frame, VIEWPORT, view);
                if before.visible_back_faces != after.visible_back_faces {
                    let difference = before.difference(&after, VIEWPORT);
                    let max_vertex_distance =
                        max_vertex_distance(&before_state.frame, &after_state.frame);
                    let before_exact = before_state
                        .angles
                        .values()
                        .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-6)
                        .count();
                    let after_exact = after_state
                        .angles
                        .values()
                        .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-6)
                        .count();
                    changed_directions += 1;
                    changed_hinges.insert((diagram.name, hinge));
                    diagram_changed.insert(hinge);
                    println!(
                        "SURFACE_180_CHANGE diagram={} edge={} kind={kind:?} direction={sign:+} before_back={} after_back={} owner_pixels={} color_pixels={} coverage_pixels={} side_pixels={} face_only_pixels={} color_bounds={:?} max_vertex_distance={max_vertex_distance:.9e} before_exact={} after_exact={}",
                        diagram.name,
                        hinge,
                        ids(&before.visible_back_faces),
                        ids(&after.visible_back_faces),
                        difference.owner_pixels,
                        difference.color_pixels,
                        difference.coverage_pixels,
                        difference.side_pixels,
                        difference.face_only_pixels,
                        difference.color_bounds,
                        before_exact,
                        after_exact,
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
    assert!(
        rank_changed_hinges.is_empty(),
        "179.999 and 180 degrees must use the same surface-rank order: {rank_changed_hinges:?}"
    );
    assert!(
        changed_hinges.len() < 79,
        "stage C must reduce the 79 previously measured endpoint changes: {changed_hinges:?}"
    );
}

#[test]
#[ignore = "folded fixture regression for the 19 stage-B rank discontinuities"]
fn surface_order_exact_endpoint_is_rank_stable_for_previous_19() {
    const PREVIOUS_RANK_CHANGES: [EdgeId; 19] = [
        125, 143, 181, 183, 297, 298, 309, 314, 352, 358, 362, 367, 380, 393, 394, 401, 402, 426,
        430,
    ];
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "folded-sample.ori3")
        .expect("the folded acceptance fixture exists");
    for hinge in PREVIOUS_RANK_CHANGES {
        assert!(diagram.hinges.iter().any(|&(edge, _)| edge == hinge));
        for sign in [1.0, -1.0] {
            let (before, after) = endpoint_frames(diagram, hinge, sign);
            let expected = surface_rank_order(&before.frame);
            assert_eq!(
                surface_rank_order(&after.frame),
                expected,
                "edge {hinge} {sign:+}"
            );

            let refreshed = solve_motion(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * 180.0,
                }],
                None,
                Some(&after.angles),
                true,
            );
            assert_eq!(
                surface_rank_order(&refreshed.result.frame),
                expected,
                "edge {hinge} {sign:+} exact refresh"
            );

            let cold = solve_motion(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * 180.0,
                }],
                None,
                None,
                true,
            );
            assert_eq!(
                surface_rank_order(&cold.result.frame),
                expected,
                "edge {hinge} {sign:+} cold exact"
            );
        }
    }
}

#[test]
fn surface_order_exact_user_zero_step_pose_reproduction() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(zero_back_user_angles().len(), 20);
    let diagram = diagram("exact-user-zero-step-pose", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    let default_camera = camera(1.0, 1.0, 1.0);
    let image = visual_image(&diagram, &frame, VIEWPORT, default_camera);

    let mut calculated_faces = render_faces(&diagram, &frame, VIEWPORT, default_camera);
    calculated_faces.sort_by_key(render_face_owner_key);
    let calculation_faces = calculated_faces
        .iter()
        .enumerate()
        .map(|(draw_order, face)| {
            serde_json::json!({
                "face": face.face,
                "draw_order": draw_order,
                "surface_rank": face.surface_rank,
                "side": face.side,
                "material_orientation": face.material_orientation,
                "back_facing": face.triangles.iter().all(|triangle| triangle.back_facing),
                "triangle_back_facing": face
                    .triangles
                    .iter()
                    .map(|triangle| triangle.back_facing)
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let fixture = serde_json::json!({
        "document": {
            "schema_version": 1,
            "paper": { "width_mm": 150.0, "height_mm": 150.0 },
            "cp": cp,
            "sequence": [],
            "display": {
                "front_color": [237, 28, 36],
                "back_color": [255, 255, 255],
                "grid_divisions": 8,
                "soft_enabled": false,
                "soft_stiffness": 0.5,
                "soft_pressure": 0.0,
                "overlap_prevention_enabled": true,
                "penetration_prevention_enabled": true,
            },
        },
        "faces": faces
            .iter()
            .map(|face| serde_json::json!({
                "id": face.id,
                "vertices": face.vertices,
                "edges": face.edges,
            }))
            .collect::<Vec<_>>(),
        "frame": frame,
        "calculation_faces": calculation_faces,
    });
    println!(
        "ZERO_BACK_PIPELINE_FIXTURE {}",
        serde_json::to_string(&fixture).expect("pipeline fixture serializes")
    );

    // 利用者指定の相互排他的RGB判定を、既定色を持つCPU rasterへそのまま適用する。
    // 背景と黒線はfill-only rasterのNoneであり、紙画素の分母へ含めない。
    let mut front = 0_u64;
    let mut back = 0_u64;
    for pixel in image.pixels.iter().flatten() {
        let [r, g, b] = if pixel.back_facing {
            [255_i16, 255_i16, 255_i16]
        } else {
            [237_i16, 28_i16, 36_i16]
        };
        let is_front = r > 140 && r - g > 40 && r - b > 40;
        let is_background = (r - 205).abs() <= 12 && (g - 200).abs() <= 12 && (b - 193).abs() <= 12;
        let is_black = r < 90 && g < 90 && b < 90;
        let is_back = !is_front
            && !is_background
            && !is_black
            && r > 150
            && (r - g).abs() < 30
            && (g - b).abs() < 30;
        front += u64::from(is_front);
        back += u64::from(is_back);
    }
    assert_eq!(front, image.red_pixels);
    assert_eq!(back, image.light_pixels);
    let total = front + back;
    assert!(total > 0);
    let back_ratio = back as f64 / total as f64;
    println!(
        "ZERO_BACK_REPRO steps=0 vertices={} edges={} creases={} front={} back={} back_ratio={:.6}% visible={} back_faces={}",
        cp.vertices.len(),
        cp.edges.len(),
        zero_back_user_angles().len(),
        front,
        back,
        back_ratio * 100.0,
        ids(&image.visible_faces),
        ids(&image.visible_back_faces),
    );

    // bad-full.pngの3D canvasは925×536 CSS px。800角の測定bufferでも水平投影へ
    // 同じaspectを入れれば、色比を保ったまま画面と同じclipping範囲を測れる。
    let screen_aspect = 925.0 / 536.0;
    let screen_image = visual_image(
        &diagram,
        &frame,
        VIEWPORT,
        camera_with_aspect(1.0, 1.0, 1.0, screen_aspect),
    );
    let screen_total = screen_image.red_pixels + screen_image.light_pixels;
    let screen_back_ratio = screen_image.light_pixels as f64 / screen_total as f64;
    println!(
        "ZERO_BACK_SCREEN_ASPECT aspect={screen_aspect:.9} front={} back={} back_ratio={:.6}% visible={} back_faces={}",
        screen_image.red_pixels,
        screen_image.light_pixels,
        screen_back_ratio * 100.0,
        ids(&screen_image.visible_faces),
        ids(&screen_image.visible_back_faces),
    );
}

#[test]
fn surface_order_exact_user_warm_start_paths() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("exact-user-warm-start-paths", cp.clone(), 1.0, 1.0);
    let final_angles = zero_back_user_angles();
    let direct_frame = zero_back_user_frame(&cp, &faces);
    let default_camera = camera(1.0, 1.0, 1.0);
    let screen_camera = camera_with_aspect(1.0, 1.0, 1.0, 925.0 / 536.0);

    let face_pixels = |image: &VisualImage| {
        let mut counts = BTreeMap::<FaceId, [u64; 2]>::new();
        for pixel in image.pixels.iter().flatten() {
            let count = counts.entry(pixel.face).or_default();
            count[usize::from(pixel.back_facing)] += 1;
        }
        counts
    };
    let direct_image = visual_image(&diagram, &direct_frame, VIEWPORT, default_camera);
    let direct_total = direct_image.red_pixels + direct_image.light_pixels;
    println!(
        "WARMSTART_BASELINE {}",
        serde_json::to_string(&serde_json::json!({
            "method": "direct-final-angle-propagation",
            "stages": 1,
            "front": direct_image.red_pixels,
            "back": direct_image.light_pixels,
            "back_ratio_percent": direct_image.light_pixels as f64 / direct_total as f64 * 100.0,
            "face_pixels_front_back": face_pixels(&direct_image),
        }))
        .expect("warm-start baseline serializes")
    );

    let mut measured = 0_usize;
    for path in [
        ZeroBackWarmPath::Active36WithPreferred,
        ZeroBackWarmPath::Edge36Only,
        ZeroBackWarmPath::AllHardControl,
    ] {
        for stages in [1_usize, 10, 50, 200] {
            let result = zero_back_warm_solve(&cp, &faces, path, stages);
            let square = visual_image(&diagram, &result.frame, VIEWPORT, default_camera);
            let screen = visual_image(&diagram, &result.frame, VIEWPORT, screen_camera);
            let square_total = square.red_pixels + square.light_pixels;
            let screen_total = screen.red_pixels + screen.light_pixels;
            assert!(square_total > 0 && screen_total > 0);

            let mut maximum_angle_delta = 0.0_f64;
            let mut angle_comparison = final_angles
                .iter()
                .map(|(&hinge, &target)| {
                    let actual = result.angles.get(&hinge).copied().unwrap_or(0.0);
                    let delta = (actual - target + 180.0).rem_euclid(360.0) - 180.0;
                    maximum_angle_delta = maximum_angle_delta.max(delta.abs());
                    serde_json::json!({
                        "hinge": hinge,
                        "target": target,
                        "actual": actual,
                        "canonical_delta": delta,
                    })
                })
                .collect::<Vec<_>>();
            angle_comparison.sort_by_key(|entry| entry["hinge"].as_u64().unwrap_or_default());

            let square_face_pixels = face_pixels(&square);
            let mut maximum_vertex_delta = 0.0_f64;
            let mut face_comparison = Vec::with_capacity(direct_frame.faces.len());
            for reference in &direct_frame.faces {
                let candidate = result
                    .frame
                    .faces
                    .iter()
                    .find(|face| face.face == reference.face)
                    .expect("warm-start frame contains every reference face");
                assert_eq!(reference.polygon.len(), candidate.polygon.len());
                let face_vertex_delta = reference
                    .polygon
                    .iter()
                    .zip(&candidate.polygon)
                    .map(|(left, right)| {
                        left.iter()
                            .zip(right)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f64>()
                            .sqrt()
                    })
                    .fold(0.0_f64, f64::max);
                maximum_vertex_delta = maximum_vertex_delta.max(face_vertex_delta);
                face_comparison.push(serde_json::json!({
                    "face": reference.face,
                    "direct_surface_rank": reference.surface_rank,
                    "warm_surface_rank": candidate.surface_rank,
                    "direct_mirrored": reference.mirrored,
                    "warm_mirrored": candidate.mirrored,
                    "max_vertex_delta": face_vertex_delta,
                    "front_pixels": square_face_pixels
                        .get(&reference.face)
                        .map_or(0, |counts| counts[0]),
                    "back_pixels": square_face_pixels
                        .get(&reference.face)
                        .map_or(0, |counts| counts[1]),
                }));
            }

            println!(
                "WARMSTART_SUMMARY method={} stages={} square_front={} square_back={} square_back_ratio={:.9}% screen_front={} screen_back={} screen_back_ratio={:.9}% converged={} closure_rms={:.3e} seam={:.3e} intersects={} contact={} max_angle_delta={:.9} max_vertex_delta={:.9}",
                path.label(),
                stages,
                square.red_pixels,
                square.light_pixels,
                square.light_pixels as f64 / square_total as f64 * 100.0,
                screen.red_pixels,
                screen.light_pixels,
                screen.light_pixels as f64 / screen_total as f64 * 100.0,
                result.converged,
                result.closure_rms,
                result.max_seam_gap,
                result.self_intersects,
                result.contact_detected,
                maximum_angle_delta,
                maximum_vertex_delta,
            );
            println!(
                "WARMSTART_CASE {}",
                serde_json::to_string(&serde_json::json!({
                    "method": path.label(),
                    "stages": stages,
                    "square": {
                        "front": square.red_pixels,
                        "back": square.light_pixels,
                        "back_ratio_percent": square.light_pixels as f64 / square_total as f64 * 100.0,
                        "visible_faces": square.visible_faces,
                        "visible_back_faces": square.visible_back_faces,
                    },
                    "screen_aspect": {
                        "front": screen.red_pixels,
                        "back": screen.light_pixels,
                        "back_ratio_percent": screen.light_pixels as f64 / screen_total as f64 * 100.0,
                        "visible_faces": screen.visible_faces,
                        "visible_back_faces": screen.visible_back_faces,
                    },
                    "solve": {
                        "converged": result.converged,
                        "closure_rms": result.closure_rms,
                        "iterations": result.iterations,
                        "contact_detected": result.contact_detected,
                        "self_intersects": result.self_intersects,
                        "max_seam_gap": result.max_seam_gap,
                        "maximum_final_angle_delta_deg": maximum_angle_delta,
                        "maximum_direct_frame_vertex_delta": maximum_vertex_delta,
                    },
                    "angles": angle_comparison,
                    "faces": face_comparison,
                }))
                .expect("warm-start measurement serializes")
            );
            measured += 1;
        }
    }
    assert_eq!(measured, 12, "three paths times four stage counts");
}

/// 利用者が実際に表示する20角度の形を、固定の36方位角 x 17仰角で測る常設検査。
/// 旧検査はedge36だけをwarm solveしたほぼ展開状態を測り、裏53,762,620画素のうち
/// 46,271,965画素に表の覆いが無かったため、612方向で裏0という条件の対象として誤っていた。
/// この検査は `zero_back_user_frame` へ対象を直し、表triangleが同じpixelを覆う裏だけを
/// 所有者判定の失敗とする。表の覆いが無い2画素は幾何的露出として座標ごと固定する。
#[test]
fn surface_order_user_frame_has_only_expected_geometric_exposure_from_all_612_directions() {
    type Exposure = (i32, i32, usize, usize, FaceId);
    // (azimuth, elevation, x, y, owner face)。CPU rasterの座標原点は左上。
    const EXPECTED_GEOMETRIC_EXPOSURES: [Exposure; 2] =
        [(40, 60, 7, 370, 13), (250, -20, 673, 206, 2)];

    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("user-frame-zero-612", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(frame.faces.len(), 16);

    let mut measured_directions = 0_usize;
    let mut total_front_pixels = 0_u64;
    let mut raw_directions_with_back = 0_usize;
    let mut raw_back_pixels = 0_u64;
    let mut directions_with_geometric_exposure = 0_usize;
    // 合否に使う `directions_with_back` は、表の覆いがあるのに裏ownerとなった方向数。
    let mut directions_with_back = 0_usize;
    let mut geometric_exposures = BTreeSet::<Exposure>::new();
    let mut unexpected_back_pixels = BTreeSet::<Exposure>::new();

    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &frame, VIEWPORT, view);
            let (front, back) = classified_fill_counts(&image);
            assert_eq!(front, image.red_pixels);
            assert_eq!(back, image.light_pixels);
            measured_directions += 1;
            total_front_pixels += front;
            if back == 0 {
                continue;
            }

            raw_directions_with_back += 1;
            raw_back_pixels += back;
            let mut front_coverage = vec![false; VIEWPORT * VIEWPORT];
            for face in render_faces(&diagram, &frame, VIEWPORT, view) {
                for triangle in face
                    .triangles
                    .iter()
                    .filter(|triangle| !triangle.back_facing)
                {
                    rasterize(
                        |pixel, _depth| front_coverage[pixel] = true,
                        triangle,
                        VIEWPORT,
                    );
                }
            }

            let mut has_geometric_exposure = false;
            let mut has_unexpected_back = false;
            for (pixel_index, pixel) in image.pixels.iter().enumerate() {
                let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
                    continue;
                };
                let exposure = (
                    azimuth_deg,
                    elevation_deg,
                    pixel_index % VIEWPORT,
                    pixel_index / VIEWPORT,
                    owner.face,
                );
                if front_coverage[pixel_index] {
                    unexpected_back_pixels.insert(exposure);
                    has_unexpected_back = true;
                } else {
                    geometric_exposures.insert(exposure);
                    has_geometric_exposure = true;
                }
            }
            directions_with_geometric_exposure += usize::from(has_geometric_exposure);
            directions_with_back += usize::from(has_unexpected_back);
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    println!(
        "ZERO612_SUMMARY directions={} total_front_pixels={} raw_directions_with_back={} raw_back_pixels={} directions_with_geometric_exposure={} geometric_exposure_pixels={} directions_with_back={} unexpected_back_pixels={} geometric_exposures={geometric_exposures:?} unexpected={unexpected_back_pixels:?}",
        measured_directions,
        total_front_pixels,
        raw_directions_with_back,
        raw_back_pixels,
        directions_with_geometric_exposure,
        geometric_exposures.len(),
        directions_with_back,
        unexpected_back_pixels.len(),
    );
    assert_eq!(
        directions_with_back, 0,
        "all covered back pixels must be eliminated: {unexpected_back_pixels:?}"
    );
    assert!(unexpected_back_pixels.is_empty());
    assert_eq!(raw_back_pixels as usize, geometric_exposures.len());
    assert_eq!(geometric_exposures.len(), 2);
    assert_eq!(
        geometric_exposures,
        BTreeSet::from(EXPECTED_GEOMETRIC_EXPOSURES),
        "geometric exposure pixels changed"
    );
}

/// 利用者の画面で紙の裏が 66.88% 見えていた `live-frame` の姿勢を、同じ
/// 36方位角 x 17仰角 = 612方向で測る常設検査。
///
/// この姿勢では16面すべてがほぼ同じ平面に重なるため、どの面が見えるかは
/// `surface_rank` だけが決める。修正前は180°の折り目15本のうち7本が折り目の
/// 向きに反しており、612方向**すべて**で裏が出ていた(実測 裏40.38%)。
///
/// 修正後に残る裏の画素は、612方向 x 800x800 の合計 22,442,018 表画素に対して
/// **2画素だけ**で、いずれも面の輪郭上に孤立した1画素である(上下左右の隣接画素は
/// すべて表)。面11・面10の投影多角形の縁がその1画素にだけ掛かった、CPU rasterの
/// 境界画素であり、まとまった面が裏を向いている状態ではない。
/// 隣接画素まで含めて固定するので、面が1枚でも裏返れば必ず落ちる。
#[test]
fn surface_order_live_frame_has_no_back_pixels_from_all_612_directions() {
    type Exposure = (i32, i32, usize, usize, FaceId);
    // (方位角, 仰角, x, y, 面番号)。CPU rasterの座標原点は左上。
    const EXPECTED_BOUNDARY_PIXELS: [Exposure; 2] =
        [(20, 20, 730, 255, 11), (140, -70, 163, 249, 10)];

    let cp = live_frame_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("live-frame-612", cp.clone(), 1.0, 1.0);
    let frame = live_frame_frame(&cp, &faces);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(frame.faces.len(), 16);

    let mut measured_directions = 0_usize;
    let mut total_front_pixels = 0_u64;
    let mut raw_back_pixels = 0_u64;
    let mut raw_directions_with_back = 0_usize;
    let mut directions_with_back = 0_usize;
    let mut geometric_exposures = BTreeSet::<Exposure>::new();
    let mut unexpected_back_pixels = BTreeSet::<Exposure>::new();

    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &frame, VIEWPORT, view);
            let (front, back) = classified_fill_counts(&image);
            assert_eq!(front, image.red_pixels);
            assert_eq!(back, image.light_pixels);
            measured_directions += 1;
            total_front_pixels += front;
            if back == 0 {
                continue;
            }
            raw_directions_with_back += 1;
            raw_back_pixels += back;
            directions_with_back += 1;
            for (pixel_index, pixel) in image.pixels.iter().enumerate() {
                let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
                    continue;
                };
                let x = pixel_index % VIEWPORT;
                let y = pixel_index / VIEWPORT;
                let exposure = (azimuth_deg, elevation_deg, x, y, owner.face);
                // 上下左右の隣接画素に裏が1つでもあれば、輪郭の1画素ではなく
                // 面が裏返って見えている領域である。
                let isolated = [(-1_i32, 0_i32), (1, 0), (0, -1), (0, 1)]
                    .into_iter()
                    .all(|(dx, dy)| {
                        let (Some(nx), Some(ny)) =
                            (x.checked_add_signed(dx as isize), y.checked_add_signed(dy as isize))
                        else {
                            return true;
                        };
                        if nx >= VIEWPORT || ny >= VIEWPORT {
                            return true;
                        }
                        image.pixels[ny * VIEWPORT + nx]
                            .is_none_or(|neighbour| !neighbour.back_facing)
                    });
                if isolated {
                    geometric_exposures.insert(exposure);
                } else {
                    unexpected_back_pixels.insert(exposure);
                }
            }
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    println!(
        "LIVE612_SUMMARY directions={measured_directions} total_front_pixels={total_front_pixels} raw_directions_with_back={raw_directions_with_back} raw_back_pixels={raw_back_pixels} isolated_boundary_pixels={} directions_with_back={directions_with_back} clustered_back_pixels={} boundary={geometric_exposures:?} clustered={unexpected_back_pixels:?}",
        geometric_exposures.len(),
        unexpected_back_pixels.len(),
    );
    assert!(total_front_pixels > 0, "紙が1画素も描かれていない");
    assert!(
        unexpected_back_pixels.is_empty(),
        "隣接画素まで裏になっている。面がまとまって裏返っている: {unexpected_back_pixels:?}"
    );
    assert_eq!(raw_back_pixels as usize, geometric_exposures.len());
    assert_eq!(
        geometric_exposures,
        BTreeSet::from(EXPECTED_BOUNDARY_PIXELS),
        "輪郭の裏画素が変わった"
    );
    // 修正前は612方向すべてで裏が出ていた。裏の割合の上限を実測値で固定する。
    assert!(
        raw_back_pixels * 1_000_000 < total_front_pixels,
        "裏 {raw_back_pixels} / 表 {total_front_pixels}"
    );
}

/// 上の612方向検査で裏になった2画素を、その画素を覆う全triangleと一緒に並べる。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn live_frame_boundary_back_pixel_detail() {
    let cp = live_frame_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("live-frame-612", cp.clone(), 1.0, 1.0);
    let frame = live_frame_frame(&cp, &faces);
    for (azimuth_deg, elevation_deg, target_x, target_y) in
        [(20_i32, 20_i32, 730_usize, 255_usize), (140, -70, 163, 249)]
    {
        let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
        let mut rendered = render_faces(&diagram, &frame, VIEWPORT, view);
        rendered.sort_by_key(render_face_owner_key);
        let target = target_y * VIEWPORT + target_x;
        println!("--- az={azimuth_deg} el={elevation_deg} pixel=({target_x},{target_y}) ---");
        let image = visual_image(&diagram, &frame, VIEWPORT, view);
        for dy in -2_i32..=2 {
            let row = (-2_i32..=2)
                .map(|dx| {
                    let x = target_x as i32 + dx;
                    let y = target_y as i32 + dy;
                    match image.pixels[y as usize * VIEWPORT + x as usize] {
                        None => "----".to_string(),
                        Some(pixel) => format!(
                            "{}{}",
                            if pixel.back_facing { "B" } else { "F" },
                            pixel.face
                        ),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("  neighbours dy={dy:+}: {row}");
        }
        for face in &rendered {
            for (index, triangle) in face.triangles.iter().enumerate() {
                let mut hit = None;
                rasterize(
                    |pixel, depth| {
                        if pixel == target {
                            hit = Some(depth);
                        }
                    },
                    triangle,
                    VIEWPORT,
                );
                if let Some(depth) = hit {
                    println!(
                        "  face={} rank={} side={} group={} back_facing={} tri={index} depth={depth:.9} key={:?}",
                        face.face,
                        face.surface_rank,
                        face.side,
                        face.coplanar_group,
                        triangle.back_facing,
                        render_face_owner_key(face),
                    );
                }
            }
        }
    }
}

#[test]
fn surface_order_user_pose_az320_el20_reports_owner_candidates() {
    #[derive(Clone, Copy)]
    struct Cover {
        draw_order: usize,
        triangle: usize,
        depth: f32,
        depth_code: u32,
        back_facing: bool,
    }

    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("user-pose-owner-diagnosis", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    println!(
        "ZERO_OWNER_FRAME_LAYERS {:?}",
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.layer, face.surface_rank))
            .collect::<Vec<_>>()
    );
    let view = camera_from_orbit_angles(1.0, 1.0, 320, 20);
    let image = visual_image(&diagram, &frame, VIEWPORT, view);
    let mut rendered = render_faces(&diagram, &frame, VIEWPORT, view);
    rendered.sort_by_key(render_face_owner_key);

    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    let back_mask = image
        .pixels
        .iter()
        .map(|pixel| pixel.is_some_and(|pixel| pixel.back_facing))
        .collect::<Vec<_>>();
    let mut covers = (0..VIEWPORT * VIEWPORT)
        .map(|_| Vec::<Cover>::new())
        .collect::<Vec<_>>();
    for (draw_order, face) in rendered.iter().enumerate() {
        for (triangle_index, triangle) in face.triangles.iter().enumerate() {
            rasterize(
                |pixel, depth| {
                    if !back_mask[pixel] {
                        return;
                    }
                    covers[pixel].push(Cover {
                        draw_order,
                        triangle: triangle_index,
                        depth,
                        depth_code: (depth.clamp(0.0, 1.0) * max_depth_code as f32).round() as u32,
                        back_facing: triangle.back_facing,
                    });
                },
                triangle,
                VIEWPORT,
            );
        }
    }

    let mut diagnosed_back_pixels = 0_u64;
    let mut no_front_at_pixel = 0_u64;
    let mut front_exists_but_farther = 0_u64;
    let mut lost_to_eligible_front = 0_u64;
    let mut same_side = 0_u64;
    let mut split_side = 0_u64;
    let mut adjacent_front_covering_current = 0_u64;
    let mut adjacent_front_not_covering_current = 0_u64;
    let mut pixels_with_adjacent_front_covering_current = 0_u64;
    let mut pixels_with_only_adjacent_front_not_covering_current = 0_u64;
    let mut winner_front_pairs = BTreeMap::<(FaceId, FaceId, u32, u32, i64, i64), u64>::new();
    let mut nearest_front_pairs = BTreeMap::<(FaceId, FaceId, u32), u64>::new();
    let mut adjacent_cover_pairs = BTreeMap::<(FaceId, FaceId, u32), u64>::new();

    for (pixel_index, pixel) in image.pixels.iter().enumerate() {
        let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
            continue;
        };
        diagnosed_back_pixels += 1;
        let pixel_covers = &covers[pixel_index];
        assert!(
            !pixel_covers.is_empty(),
            "owner pixel must have a raster cover"
        );
        let minimum_depth_code = pixel_covers
            .iter()
            .map(|cover| cover.depth_code)
            .min()
            .expect("a back pixel has at least one cover");
        let minimum_depth = minimum_depth_code as f32 / max_depth_code as f32;
        let tolerance = DEPTH_TIE_CODES as f32 / max_depth_code as f32;
        // productionのvisual_imageと同じく、量子化されたnearestに対して
        // fragmentのraw f32 depthを比較する。丸め後code差だけでは境界がずれる。
        let eligible = |cover: &Cover| cover.depth - minimum_depth <= tolerance;
        let expected_owner = pixel_covers
            .iter()
            .filter(|cover| eligible(cover))
            .max_by_key(|cover| cover.draw_order)
            .expect("the nearest cover is always eligible");
        assert_eq!(rendered[expected_owner.draw_order].face, owner.face);
        assert_eq!(expected_owner.back_facing, owner.back_facing);
        let best_front_any = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing)
            .max_by_key(|cover| cover.draw_order);
        let nearest_front = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing)
            .min_by(|left, right| {
                left.depth
                    .total_cmp(&right.depth)
                    .then(right.draw_order.cmp(&left.draw_order))
            });
        let best_front_eligible = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing && eligible(cover))
            .max_by_key(|cover| cover.draw_order);
        if let Some(front) = nearest_front {
            *nearest_front_pairs
                .entry((
                    owner.face,
                    rendered[front.draw_order].face,
                    front.depth_code.saturating_sub(minimum_depth_code),
                ))
                .or_default() += 1;
        }

        match (best_front_any, best_front_eligible) {
            (None, _) => no_front_at_pixel += 1,
            (Some(_), None) => front_exists_but_farther += 1,
            (Some(_), Some(front)) => {
                lost_to_eligible_front += 1;
                let winner = rendered
                    .iter()
                    .find(|face| face.face == owner.face)
                    .expect("the owner face remains in the rendered list");
                let front_face = &rendered[front.draw_order];
                if winner.side == front_face.side {
                    same_side += 1;
                } else {
                    split_side += 1;
                }
                *winner_front_pairs
                    .entry((
                        winner.face,
                        front_face.face,
                        winner.surface_rank,
                        front_face.surface_rank,
                        winner.side,
                        front_face.side,
                    ))
                    .or_default() += 1;
            }
        }

        let x = pixel_index % VIEWPORT;
        let y = pixel_index / VIEWPORT;
        let mut adjacent = Vec::new();
        let mut has_adjacent_front_covering_current = false;
        let mut has_adjacent_front_not_covering_current = false;
        for (dx, dy) in [(-1_isize, 0_isize), (1, 0), (0, -1), (0, 1)] {
            let nx = x.checked_add_signed(dx);
            let ny = y.checked_add_signed(dy);
            let Some((nx, ny)) = nx
                .zip(ny)
                .filter(|(nx, ny)| *nx < VIEWPORT && *ny < VIEWPORT)
            else {
                continue;
            };
            let neighbor_index = ny * VIEWPORT + nx;
            let neighbor = image.pixels[neighbor_index];
            let covers_current = neighbor.is_some_and(|neighbor| {
                pixel_covers
                    .iter()
                    .any(|cover| rendered[cover.draw_order].face == neighbor.face)
            });
            if let Some(neighbor) = neighbor.filter(|neighbor| !neighbor.back_facing) {
                if covers_current {
                    adjacent_front_covering_current += 1;
                    has_adjacent_front_covering_current = true;
                    if let Some(cover) = pixel_covers
                        .iter()
                        .filter(|cover| rendered[cover.draw_order].face == neighbor.face)
                        .min_by(|left, right| left.depth.total_cmp(&right.depth))
                    {
                        *adjacent_cover_pairs
                            .entry((
                                owner.face,
                                neighbor.face,
                                cover.depth_code.saturating_sub(minimum_depth_code),
                            ))
                            .or_default() += 1;
                    }
                } else {
                    adjacent_front_not_covering_current += 1;
                    has_adjacent_front_not_covering_current = true;
                }
                adjacent.push(serde_json::json!({
                    "dx": dx,
                    "dy": dy,
                    "face": neighbor.face,
                    "back_facing": neighbor.back_facing,
                    "covers_current_pixel": covers_current,
                }));
            }
        }
        if has_adjacent_front_covering_current {
            pixels_with_adjacent_front_covering_current += 1;
        } else if has_adjacent_front_not_covering_current {
            pixels_with_only_adjacent_front_not_covering_current += 1;
        }

        let candidate_json = pixel_covers
            .iter()
            .map(|cover| {
                let face = &rendered[cover.draw_order];
                serde_json::json!({
                    "face": face.face,
                    "surface_rank": face.surface_rank,
                    "side": face.side,
                    "side_times_surface_rank": face.side * i64::from(face.surface_rank),
                    "material_orientation": face.material_orientation,
                    "triangle": cover.triangle,
                    "back_facing": cover.back_facing,
                    "depth_code": cover.depth_code,
                    "minimum_depth_delta_codes": cover.depth_code.saturating_sub(minimum_depth_code),
                    "raw_minimum_depth_delta_codes":
                        (cover.depth - minimum_depth) * max_depth_code as f32,
                    "eligible": eligible(cover),
                    "draw_order": cover.draw_order,
                })
            })
            .collect::<Vec<_>>();
        let front_json = |cover: &Cover| {
            let face = &rendered[cover.draw_order];
            serde_json::json!({
                "face": face.face,
                "surface_rank": face.surface_rank,
                "side": face.side,
                "side_times_surface_rank": face.side * i64::from(face.surface_rank),
                "material_orientation": face.material_orientation,
                "depth_code": cover.depth_code,
                "minimum_depth_delta_codes": cover.depth_code.saturating_sub(minimum_depth_code),
                "raw_minimum_depth_delta_codes":
                    (cover.depth - minimum_depth) * max_depth_code as f32,
                "draw_order": cover.draw_order,
            })
        };
        if std::env::var_os("ZERO_OWNER_VERBOSE").is_some() {
            println!(
                "ZERO_OWNER_PIXEL {}",
                serde_json::to_string(&serde_json::json!({
                    "x": x,
                    "y": y,
                    "winner": {
                        "face": owner.face,
                        "back_facing": owner.back_facing,
                    },
                    "minimum_depth_code": minimum_depth_code,
                    "best_front_any": best_front_any.map(front_json),
                    "best_front_eligible": best_front_eligible.map(front_json),
                    "candidates": candidate_json,
                    "adjacent_front_winners": adjacent,
                }))
                .expect("owner pixel diagnosis serializes")
            );
        }
    }

    assert_eq!(diagnosed_back_pixels, image.light_pixels);
    assert_eq!(
        no_front_at_pixel + front_exists_but_farther + lost_to_eligible_front,
        diagnosed_back_pixels,
    );
    println!(
        "ZERO_OWNER_SUMMARY azimuth_deg=320 elevation_deg=20 viewport={} front_pixels={} back_pixels={} no_front_at_pixel={} front_exists_but_farther={} lost_to_eligible_front={} same_side={} split_side={} adjacent_front_covering_current={} adjacent_front_not_covering_current={} pixels_with_adjacent_front_covering_current={} pixels_with_only_adjacent_front_not_covering_current={} winner_front_pairs={winner_front_pairs:?} nearest_front_pairs={nearest_front_pairs:?} adjacent_cover_pairs={adjacent_cover_pairs:?}",
        VIEWPORT,
        image.red_pixels,
        image.light_pixels,
        no_front_at_pixel,
        front_exists_but_farther,
        lost_to_eligible_front,
        same_side,
        split_side,
        adjacent_front_covering_current,
        adjacent_front_not_covering_current,
        pixels_with_adjacent_front_covering_current,
        pixels_with_only_adjacent_front_not_covering_current,
    );
}

/// 裏画素を「表の覆いがある(所有者判定の失敗)」と「表の覆いが無い(幾何的露出)」へ分ける。
fn covered_back_counts(diagram: &Diagram, frame: &Frame3D, view: Camera) -> (u64, u64, u64) {
    let image = visual_image(diagram, frame, VIEWPORT, view);
    let (front, back) = classified_fill_counts(&image);
    if back == 0 {
        return (front, 0, 0);
    }
    let mut front_coverage = vec![false; VIEWPORT * VIEWPORT];
    for face in render_faces(diagram, frame, VIEWPORT, view) {
        for triangle in face
            .triangles
            .iter()
            .filter(|triangle| !triangle.back_facing)
        {
            rasterize(
                |pixel, _depth| front_coverage[pixel] = true,
                triangle,
                VIEWPORT,
            );
        }
    }
    let mut covered = 0_u64;
    for (pixel_index, pixel) in image.pixels.iter().enumerate() {
        if pixel.is_some_and(|pixel| pixel.back_facing) && front_coverage[pixel_index] {
            covered += 1;
        }
    }
    (front, back, covered)
}

/// 深度のtie窓と組符号を一切使わず、純粋な最前深度だけで所有者を決めた場合の表・裏。
/// 現行規則との差が、tie解決(surface_rank)に由来する裏画素の量になる。
fn strict_nearest_counts(diagram: &Diagram, frame: &Frame3D, view: Camera) -> (u64, u64) {
    let mut faces = render_faces(diagram, frame, VIEWPORT, view);
    faces.sort_by_key(render_face_owner_key);
    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    let mut nearest = vec![f32::INFINITY; VIEWPORT * VIEWPORT];
    let mut owner_back = vec![None::<bool>; VIEWPORT * VIEWPORT];
    for face in &faces {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let code = (depth.clamp(0.0, 1.0) * max_depth_code as f32).round();
                    if code < nearest[pixel] {
                        nearest[pixel] = code;
                        owner_back[pixel] = Some(triangle.back_facing);
                    }
                },
                triangle,
                VIEWPORT,
            );
        }
    }
    let mut front = 0_u64;
    let mut back = 0_u64;
    for owner in owner_back.into_iter().flatten() {
        if owner {
            back += 1;
        } else {
            front += 1;
        }
    }
    (front, back)
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1b_edge36_pose_back_pixels_are_tie_or_geometry() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1b-edge36-pose", cp.clone(), 1.0, 1.0);
    let default_camera = camera(1.0, 1.0, 1.0);
    for (pose, final_angles) in [
        ("angles-now", zero_back_user_angles_now()),
        ("angles-old", zero_back_user_angles()),
    ] {
        let result = zero_back_warm_solve_to(
            &cp,
            &faces,
            &final_angles,
            ZeroBackWarmPath::Edge36Only,
            200,
        );
        let (front, back, covered) = covered_back_counts(&diagram, &result.frame, default_camera);
        let (strict_front, strict_back) =
            strict_nearest_counts(&diagram, &result.frame, default_camera);
        let angles = result
            .angles
            .iter()
            .map(|(&hinge, &angle)| (hinge, angle))
            .collect::<BTreeMap<_, _>>();
        println!(
            "STAGE1B pose={pose} rule_front={front} rule_back={back} rule_covered_back={covered} strict_front={strict_front} strict_back={strict_back} angles={:?}",
            angles
                .iter()
                .map(|(hinge, angle)| format!("{hinge}:{angle:.3}"))
                .collect::<Vec<_>>(),
        );
    }
}

/// `ORI3_STAGE1_IMAGE_DIR` を指定して実行したときだけ、CPU rasterの見た目をPPMで書き出す。
/// 既定では何も書かないので、追跡対象のファイルは変化しない。
#[test]
#[ignore = "調査用。ORI3_STAGE1_IMAGE_DIR を指定したときだけ画像を書く"]
fn stage1d_write_pose_images() {
    let Ok(directory) = std::env::var("ORI3_STAGE1_IMAGE_DIR") else {
        println!("STAGE1D skipped: ORI3_STAGE1_IMAGE_DIR is unset");
        return;
    };
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1d-images", cp.clone(), 1.0, 1.0);
    let edge36 = zero_back_warm_solve_to(
        &cp,
        &faces,
        &zero_back_user_angles(),
        ZeroBackWarmPath::Edge36Only,
        200,
    );
    let user = zero_back_user_frame(&cp, &faces);
    for (name, frame) in [("edge36-only-200", &edge36.frame), ("user-pose", &user)] {
        for (view_name, view) in [
            ("front", camera(1.0, 1.0, 1.0)),
            ("back", camera(1.0, 1.0, -1.0)),
        ] {
            let image = visual_image(&diagram, frame, VIEWPORT, view);
            let mut bytes = format!("P6\n{VIEWPORT} {VIEWPORT}\n255\n").into_bytes();
            for pixel in &image.pixels {
                let rgb = match pixel {
                    None => [207_u8, 203, 194],
                    Some(pixel) if pixel.back_facing => [255, 255, 255],
                    Some(_) => [237, 28, 36],
                };
                bytes.extend_from_slice(&rgb);
            }
            let path = format!("{directory}/stage1d-{name}-{view_name}.ppm");
            std::fs::write(&path, bytes).expect("write diagnostic image");
            println!("STAGE1D wrote {path}");
        }
    }
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1c_edge36_pose_rule_versus_strict_over_612_directions() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1c-edge36-pose-612", cp.clone(), 1.0, 1.0);
    let result = zero_back_warm_solve_to(
        &cp,
        &faces,
        &zero_back_user_angles(),
        ZeroBackWarmPath::Edge36Only,
        200,
    );
    let mut directions = 0_usize;
    let mut rule_back_total = 0_u64;
    let mut strict_back_total = 0_u64;
    let mut worse_directions = 0_usize;
    let mut worst = (0_i64, 0_i32, 0_i32);
    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &result.frame, VIEWPORT, view);
            let (_, rule_back) = classified_fill_counts(&image);
            let (_, strict_back) = strict_nearest_counts(&diagram, &result.frame, view);
            directions += 1;
            rule_back_total += rule_back;
            strict_back_total += strict_back;
            let difference = rule_back as i64 - strict_back as i64;
            if difference > 0 {
                worse_directions += 1;
                if difference > worst.0 {
                    worst = (difference, azimuth_deg, elevation_deg);
                }
            }
        }
    }
    println!(
        "STAGE1C directions={directions} rule_back_total={rule_back_total} strict_back_total={strict_back_total} directions_rule_worse_than_strict={worse_directions} worst_excess={} at_az={} el={}",
        worst.0, worst.1, worst.2,
    );
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1_surface_rank_direct_versus_warm_start() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1-rank-comparison", cp.clone(), 1.0, 1.0);
    let default_camera = camera(1.0, 1.0, 1.0);

    for (pose, final_angles) in [
        ("angles-now", zero_back_user_angles_now()),
        ("angles-old", zero_back_user_angles()),
    ] {
        let mut methods: Vec<(String, Frame3D, HashMap<EdgeId, f64>)> = Vec::new();

        // A0: 最終角をそのまま propagate して組み立てる基準形。
        let direct =
            zero_back_apply_overlap(&cp, &faces, {
                let folded = propagate(&cp, &faces, &final_angles);
                to_frame3d(&cp, &faces, &folded)
            });
        methods.push(("A0-propagate-direct".to_string(), direct, final_angles.clone()));

        // A1: 最終角をいきなり全部hardで1回solveする(warm無し)。
        let mut hard = final_angles
            .iter()
            .map(|(&hinge, &target_angle_deg)| Driver {
                hinge,
                target_angle_deg,
            })
            .collect::<Vec<_>>();
        hard.sort_unstable_by_key(|driver| driver.hinge);
        let cold = solve_motion(&cp, &faces, &hard, None, None, true);
        methods.push((
            "A1-solve-cold-one-shot".to_string(),
            zero_back_apply_overlap(&cp, &faces, cold.result.frame),
            cold.result.angles,
        ));

        // B: 0度から200段で少しずつ動かし、直前の解をwarm startにする。
        for path in [
            ZeroBackWarmPath::AllHardControl,
            ZeroBackWarmPath::Active36WithPreferred,
            ZeroBackWarmPath::Edge36Only,
        ] {
            let result = zero_back_warm_solve_to(&cp, &faces, &final_angles, path, 200);
            methods.push((
                format!("B-{}-200stages", path.label()),
                result.frame,
                result.angles,
            ));
        }

        let mut reference_ranks: Option<BTreeMap<FaceId, u32>> = None;
        for (label, frame, angles) in &methods {
            let ranks = frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>();
            let mismatched = reference_ranks.as_ref().map(|reference| {
                reference
                    .iter()
                    .filter(|(face, rank)| ranks.get(face) != Some(rank))
                    .map(|(&face, _)| face)
                    .collect::<Vec<_>>()
            });
            let (front, back, covered) = covered_back_counts(&diagram, frame, default_camera);
            let mut maximum_angle_delta = 0.0_f64;
            for (&hinge, &target) in &final_angles {
                let actual = angles.get(&hinge).copied().unwrap_or(0.0);
                let delta = (actual - target + 180.0).rem_euclid(360.0) - 180.0;
                maximum_angle_delta = maximum_angle_delta.max(delta.abs());
            }
            println!(
                "STAGE1 pose={pose} method={label} ranks={:?} mismatch_count={} mismatch_faces={:?} front={front} back={back} covered_back={covered} back_ratio={:.6}% max_angle_delta={maximum_angle_delta:.9}",
                ranks.values().copied().collect::<Vec<_>>(),
                mismatched.as_ref().map_or(0, Vec::len),
                mismatched.unwrap_or_default(),
                back as f64 / (front + back).max(1) as f64 * 100.0,
            );
            if reference_ranks.is_none() {
                reference_ranks = Some(ranks);
            }
        }
    }
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
            let (baseline_red, baseline_light) = match (fold, view_name) {
                ("mountain", "front") => (45_015_u64, 80_u64),
                ("mountain", "back") => (50_438, 6_244),
                ("valley", "front") => (45_777, 4_580),
                ("valley", "back") => (56_603, 79),
                _ => unreachable!("the four acceptance conditions are exhaustive"),
            };
            let baseline_ratio = baseline_red as f64 / (baseline_red + baseline_light) as f64;
            println!(
                "SURFACE_94 fold={fold} angle={angle:+.0} view={view_name} red={} light={} red_ratio={:.6}% baseline_red={} baseline_light={} baseline_red_ratio={:.6}% visible={} back_faces={}",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                baseline_red,
                baseline_light,
                baseline_ratio * 100.0,
                ids(&image.visible_faces),
                ids(&image.visible_back_faces),
            );
            assert!(image.red_pixels + image.light_pixels > 0);
            assert!(
                image.red_ratio() >= baseline_ratio,
                "{fold} {angle:+.0} degrees from the {view_name} must not regress below the measured baseline: red={} light={} ratio={:.9}% baseline={:.9}%",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                baseline_ratio * 100.0,
            );
            if fold == "valley" && view_name == "front" {
                valley_front = Some(image);
            }
        }
    }
    let valley_front = valley_front.expect("the four conditions include valley/front");
    assert!(
        valley_front.red_ratio() >= 0.909,
        "valley -94 degrees from the front must be at least 90.9% red: red={} light={}",
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

#[test]
fn surface_order_user_pose_minus_180_is_deterministic_ten_times() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("diagonal-midline-user-pose", cp.clone(), 1.0, 1.0);
    let angles = angle_surface_angles(-180.0);
    let mut expected = None::<(Vec<u8>, VisualImage)>;
    for run in 1..=10 {
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let serialized = serde_json::to_vec(&frame).expect("serialize deterministic user pose");
        let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, 1.0));
        if let Some(reference) = &expected {
            assert_eq!(
                &serialized, &reference.0,
                "user-pose frame changed on run {run}"
            );
            assert_eq!(
                &image, &reference.1,
                "user-pose visible result changed on run {run}"
            );
        } else {
            expected = Some((serialized, image));
        }
    }
    println!("SURFACE_DETERMINISM identical_runs=10 input=edge43:-180");
}

/// 決定的な擬似乱数(SplitMix64)。種を固定すれば毎回同じ姿勢の並びを作る。
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [0,1) の一様乱数。
    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn next_below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// 折り目の目標角を1本ぶん引く。ほぼ平ら(±180°付近)を意図的に厚めに混ぜる。
fn sweep_angle(random: &mut SplitMix64) -> f64 {
    let sign = if random.next_unit() < 0.5 { -1.0 } else { 1.0 };
    let magnitude = match random.next_below(10) {
        0..=5 => random.next_unit() * 180.0,
        6 | 7 => 170.0 + random.next_unit() * 9.9,
        _ => {
            const NEAR_FLAT: [f64; 6] = [179.9, 179.99, 179.999, 179.999_9, 179.999_999, 180.0];
            NEAR_FLAT[random.next_below(NEAR_FLAT.len())]
        }
    };
    sign * magnitude
}

struct SweepPose {
    label: String,
    frame: Frame3D,
    /// 解けた全ヒンジ角。独立検証の探り姿勢を作るのに使う。
    angles: HashMap<EdgeId, f64>,
    /// 重なり順をどの幾何から決めたか。
    source_label: &'static str,
    /// `stamp_motion_surface_order` の canonical 経路条件を外から作り直したもの。
    needs_canonical_path: bool,
    /// `stamp_motion_surface_order` の has_exact 条件を外から作り直したもの。
    has_exact: bool,
    /// 紙が裂けていない形かどうかを分けるための実測値。
    max_seam_gap: f64,
    self_intersects: bool,
}

/// 種を固定して `count` 通りの姿勢を作る。全て `solve_motion` の製品経路を通す。
fn sweep_poses(cp: &CreasePattern, faces: &[Face], seed: u64, count: usize) -> Vec<SweepPose> {
    let mut hinges = fold_hinges(cp, faces)
        .into_iter()
        .map(|(hinge, _)| hinge)
        .collect::<Vec<_>>();
    hinges.sort_unstable();
    let mut random = SplitMix64(seed);
    let mut poses = Vec::with_capacity(count);
    for index in 0..count {
        let mode = index % 3;
        let targets = hinges
            .iter()
            .map(|&hinge| (hinge, sweep_angle(&mut random)))
            .collect::<HashMap<EdgeId, f64>>();
        let (hard, preferred) = match mode {
            // すべての折り目を hard で指定する。
            0 => {
                let mut hard = targets
                    .iter()
                    .map(|(&hinge, &target_angle_deg)| Driver {
                        hinge,
                        target_angle_deg,
                    })
                    .collect::<Vec<_>>();
                hard.sort_unstable_by_key(|driver| driver.hinge);
                (hard, None)
            }
            // 1〜4本だけ hard、残りを preferred にする。
            1 => {
                let driven = 1 + random.next_below(4);
                let mut chosen = BTreeSet::new();
                while chosen.len() < driven {
                    chosen.insert(hinges[random.next_below(hinges.len())]);
                }
                let hard = chosen
                    .iter()
                    .map(|&hinge| Driver {
                        hinge,
                        target_angle_deg: targets[&hinge],
                    })
                    .collect::<Vec<_>>();
                let rest = targets
                    .iter()
                    .filter(|(hinge, _)| !chosen.contains(hinge))
                    .map(|(&hinge, &angle)| (hinge, angle))
                    .collect::<HashMap<_, _>>();
                (hard, Some(rest))
            }
            // 1〜3本だけ hard にして残りは自由に追従させる(preferred 無し)。
            _ => {
                let driven = 1 + random.next_below(3);
                let mut chosen = BTreeSet::new();
                while chosen.len() < driven {
                    chosen.insert(hinges[random.next_below(hinges.len())]);
                }
                let hard = chosen
                    .iter()
                    .map(|&hinge| Driver {
                        hinge,
                        target_angle_deg: targets[&hinge],
                    })
                    .collect::<Vec<_>>();
                (hard, None)
            }
        };
        // 0度から段階的に近づけ、直前の解をwarm startにする。いきなり最終角へ
        // 解くと紙が裂けた形ばかりになり、実際の操作で現れる姿勢から遠ざかる。
        const RAMP_STAGES: usize = 8;
        let mut warm = hinges
            .iter()
            .map(|&hinge| (hinge, 0.0))
            .collect::<HashMap<EdgeId, f64>>();
        let mut motion = None;
        for stage in 1..=RAMP_STAGES {
            let progress = stage as f64 / RAMP_STAGES as f64;
            let stage_hard = hard
                .iter()
                .map(|driver| Driver {
                    hinge: driver.hinge,
                    target_angle_deg: driver.target_angle_deg * progress,
                })
                .collect::<Vec<_>>();
            let stage_preferred = preferred.as_ref().map(|rest| {
                rest.iter()
                    .map(|(&hinge, &angle)| (hinge, angle * progress))
                    .collect::<HashMap<_, _>>()
            });
            motion = Some(solve_motion(
                cp,
                faces,
                &stage_hard,
                stage_preferred.as_ref(),
                Some(&warm),
                true,
            ));
            warm = motion
                .as_ref()
                .expect("the ramp just solved a stage")
                .result
                .angles
                .clone();
        }
        let motion = motion.expect("the ramp runs at least one stage");
        let max_seam_gap = ori3_rigid::max_seam_gap(cp, faces, &motion.result.frame);
        let self_intersects = ori3_rigid::self_intersects(&motion.result.frame);
        let mut canonical = preferred.clone().unwrap_or_default();
        for driver in &hard {
            canonical.insert(driver.hinge, driver.target_angle_deg);
        }
        let needs_canonical_path = canonical
            .values()
            .any(|angle| angle.abs() >= 179.999 - 1e-6);
        let has_exact = motion.result.angles.values().any(|angle| {
            (angle.abs().to_radians() - std::f64::consts::PI).abs() <= 1e-8
        });
        let mut summary = hard
            .iter()
            .map(|driver| format!("{}:{:.4}", driver.hinge, driver.target_angle_deg))
            .collect::<Vec<_>>();
        summary.sort_unstable();
        poses.push(SweepPose {
            label: format!("pose{index:03}-mode{mode}-hard[{}]", summary.join(" ")),
            source_label: surface_order_source_label(motion.surface_order.map(|order| order.source)),
            angles: motion.result.angles,
            frame: motion.result.frame,
            needs_canonical_path,
            has_exact,
            max_seam_gap,
            self_intersects,
        });
    }
    poses
}

/// 段階1: 多数の姿勢で「現行規則の裏」と「重なり順を一切使わない最前面判定の裏」を比べる。
/// 合否は付けない(§10.7.7)。数を出して段階2の判断材料にする。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1_random_pose_sweep_rule_versus_strict() {
    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSES: usize = 240;
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1-sweep", cp.clone(), 1.0, 1.0);
    let views = [
        ("default", camera(1.0, 1.0, 1.0)),
        ("back", camera(1.0, 1.0, -1.0)),
        ("az0el0", camera_from_orbit_angles(1.0, 1.0, 0, 0)),
        ("az90el30", camera_from_orbit_angles(1.0, 1.0, 90, 30)),
        ("az200el-40", camera_from_orbit_angles(1.0, 1.0, 200, -40)),
        ("az285el55", camera_from_orbit_angles(1.0, 1.0, 285, 55)),
    ];
    let faceid_order = {
        let mut order = faces.iter().map(|face| face.id).collect::<Vec<_>>();
        order.sort_unstable();
        order
    };
    let poses = sweep_poses(&cp, &faces, SEED, POSES);
    let mut worse_poses = 0_usize;
    let mut clean_poses = 0_usize;
    let mut clean_worse_poses = 0_usize;
    let mut faceid_order_poses = 0_usize;
    let mut current_depth_branch = 0_usize;
    let mut rule_grand_total = 0_u64;
    let mut strict_grand_total = 0_u64;
    for pose in &poses {
        let order = surface_rank_order(&pose.frame);
        let is_faceid_order = order == faceid_order;
        // 裂けが小さく自己交差の無い形だけを「実際に折れる形」として別に数える。
        let is_clean = pose.max_seam_gap < 1e-6 && !pose.self_intersects;
        if is_faceid_order {
            faceid_order_poses += 1;
        }
        if is_clean {
            clean_poses += 1;
        }
        if !pose.needs_canonical_path && !pose.has_exact {
            current_depth_branch += 1;
        }
        let mut worst = (0_i64, "");
        let mut rule_total = 0_u64;
        let mut strict_total = 0_u64;
        for (view_name, view) in &views {
            let (_, rule_back, _) = covered_back_counts(&diagram, &pose.frame, *view);
            let (_, strict_back) = strict_nearest_counts(&diagram, &pose.frame, *view);
            rule_total += rule_back;
            strict_total += strict_back;
            let excess = rule_back as i64 - strict_back as i64;
            if excess > worst.0 {
                worst = (excess, view_name);
            }
        }
        rule_grand_total += rule_total;
        strict_grand_total += strict_total;
        if worst.0 > 0 {
            worse_poses += 1;
            if is_clean {
                clean_worse_poses += 1;
            }
            println!(
                "STAGE1SWEEP worse {} clean={is_clean} excess={} view={} rule_back={rule_total} strict_back={strict_total} faceid_order={is_faceid_order} needs_canonical={} has_exact={} seam={:.3e} intersects={} ranks={:?}",
                pose.label,
                worst.0,
                worst.1,
                pose.needs_canonical_path,
                pose.has_exact,
                pose.max_seam_gap,
                pose.self_intersects,
                order,
            );
        }
    }
    println!(
        "STAGE1SWEEP seed={SEED:#x} poses={POSES} views={} clean_poses={clean_poses} rule_worse_than_strict_poses={worse_poses} clean_rule_worse_than_strict_poses={clean_worse_poses} faceid_order_poses={faceid_order_poses} current_depth_branch_poses={current_depth_branch} rule_back_total={rule_grand_total} strict_back_total={strict_grand_total}",
        views.len(),
    );
}

// ===========================================================================
// 「上に来るべき面」を、アプリの重なり順とは独立に幾何から求めて突き合わせる。
//
// ここでは `surface_rank` を一切読まない。紙の位置と折り目のつながりだけから
// 「完全に重なった2枚のうち、どちらが上か」を決め、アプリが使っている順と比べる。
// ===========================================================================

/// 重なりを調べるための、いまの姿勢での面の平面。
struct OverlapPlane {
    origin: V3,
    normal: V3,
    u: V3,
    v: V3,
}

fn polygon_normal3(points: &[V3]) -> Option<V3> {
    if points.len() < 3 {
        return None;
    }
    let mut normal = V3::ZERO;
    for index in 0..points.len() {
        normal += points[index].cross(points[(index + 1) % points.len()]);
    }
    (normal.length_squared() > 1e-24).then(|| normal.normalize())
}

/// 法線の向きの符号を消す。上下の比較を面の巻き方向に依存させないため。
fn canonical3(mut normal: V3) -> V3 {
    let absolute = V3::new(normal.x.abs(), normal.y.abs(), normal.z.abs());
    let component = if absolute.x >= absolute.y && absolute.x >= absolute.z {
        normal.x
    } else if absolute.y >= absolute.z {
        normal.y
    } else {
        normal.z
    };
    if component < 0.0 {
        normal = -normal;
    }
    normal
}

fn overlap_plane(points: &[V3]) -> Option<OverlapPlane> {
    let origin = *points.first()?;
    let normal = canonical3(polygon_normal3(points)?);
    let u = (0..points.len())
        .map(|index| points[(index + 1) % points.len()] - points[index])
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .normalize();
    let v = normal.cross(u).normalize();
    Some(OverlapPlane {
        origin,
        normal,
        u,
        v,
    })
}

fn project2(points: &[V3], plane: &OverlapPlane) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|point| {
            let relative = *point - plane.origin;
            [relative.dot(plane.u), relative.dot(plane.v)]
        })
        .collect()
}

fn polygon_area2(polygon: &[[f64; 2]]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
}

/// 凸多角形どうしのクリップ。三角形どうしにだけ使う。
fn clip_convex(subject: &[[f64; 2]], clip: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut output = subject.to_vec();
    let side = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0])
    };
    let winding = if polygon_area2(clip) < 0.0 { -1.0 } else { 1.0 };
    for index in 0..clip.len() {
        let start = clip[index];
        let end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        let Some(mut previous) = input.last().copied() else {
            break;
        };
        let mut previous_side = side(start, end, previous) * winding;
        for current in input {
            let current_side = side(start, end, current) * winding;
            if (previous_side >= 0.0) != (current_side >= 0.0) {
                let denominator = previous_side - current_side;
                if denominator.abs() > 1e-18 {
                    let ratio = previous_side / denominator;
                    output.push([
                        previous[0] + (current[0] - previous[0]) * ratio,
                        previous[1] + (current[1] - previous[1]) * ratio,
                    ]);
                }
            }
            if current_side >= 0.0 {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    output
}

/// 2枚の面が実面積で重なる代表点(共通平面上の2D座標)と重なり面積を返す。
fn overlap_witness(
    left: &[[f64; 2]],
    left_triangles: &[[usize; 3]],
    right: &[[f64; 2]],
    right_triangles: &[[usize; 3]],
) -> Option<([f64; 2], f64)> {
    let mut best: Option<([f64; 2], f64)> = None;
    for left_indices in left_triangles {
        let left_triangle = left_indices.map(|index| left[index]);
        for right_indices in right_triangles {
            let right_triangle = right_indices.map(|index| right[index]);
            let intersection = clip_convex(&left_triangle, &right_triangle);
            let area = polygon_area2(&intersection).abs();
            if area <= 1e-12 || intersection.len() < 3 {
                continue;
            }
            let sum = intersection
                .iter()
                .fold([0.0, 0.0], |sum, point| [sum[0] + point[0], sum[1] + point[1]]);
            let count = intersection.len() as f64;
            let center = [sum[0] / count, sum[1] / count];
            if best.is_none_or(|(_, best_area)| area > best_area) {
                best = Some((center, area));
            }
        }
    }
    best
}

/// `exact` の面上の点を材質座標へ戻し、`probe` の同じ面へ写す。
fn map_material_point(exact: &[V3], probe: &[V3], point: V3) -> Option<V3> {
    if exact.len() != probe.len() || exact.len() < 3 {
        return None;
    }
    let origin = exact[0];
    let (first, second) = (1..exact.len()).find_map(|first| {
        (first + 1..exact.len())
            .find(|&second| {
                (exact[first] - origin)
                    .cross(exact[second] - origin)
                    .length_squared()
                    > 1e-24
            })
            .map(|second| (first, second))
    })?;
    let a = exact[first] - origin;
    let b = exact[second] - origin;
    let relative = point - origin;
    let aa = a.length_squared();
    let ab = a.dot(b);
    let bb = b.length_squared();
    let determinant = aa * bb - ab * ab;
    if determinant.abs() <= 1e-24 {
        return None;
    }
    let ra = relative.dot(a);
    let rb = relative.dot(b);
    let first_weight = (ra * bb - rb * ab) / determinant;
    let second_weight = (rb * aa - ra * ab) / determinant;
    let probe_origin = probe[0];
    Some(
        probe_origin
            + (probe[first] - probe_origin) * first_weight
            + (probe[second] - probe_origin) * second_weight,
    )
}

fn frame_polygons(frame: &Frame3D) -> BTreeMap<FaceId, Vec<V3>> {
    frame
        .faces
        .iter()
        .map(|face| {
            (
                face.face,
                face.polygon
                    .iter()
                    .map(|point| V3::new(point[0], point[1], point[2]))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// いまの姿勢で「同じ平面に乗っていて、実面積で重なっている」面対。
/// ここが、カメラからの深度が並んで重なり順が表示を決める場所である。
struct CoincidentOverlap {
    left: FaceId,
    right: FaceId,
    /// 共通平面の正準法線。この向きに高いほうが「上」。
    normal: V3,
    /// 重なり領域の代表点(3D、いまの姿勢)。
    witness: V3,
    /// 2面が直接つながっている折り目(あれば)。
    shared_hinge: Option<EdgeId>,
}

/// `surface_rank` を一切読まずに、完全に重なっている面対を全て挙げる。
fn coincident_overlaps(diagram: &Diagram, frame: &Frame3D) -> Vec<CoincidentOverlap> {
    let polygons = frame_polygons(frame);
    let mut face_ids = polygons.keys().copied().collect::<Vec<_>>();
    face_ids.sort_unstable();
    let edges_of = diagram
        .faces
        .iter()
        .map(|face| (face.id, face.edges.iter().copied().collect::<BTreeSet<_>>()))
        .collect::<BTreeMap<_, _>>();
    let mut overlaps = Vec::new();
    for (left_index, &left) in face_ids.iter().enumerate() {
        for &right in &face_ids[left_index + 1..] {
            let left_points = &polygons[&left];
            let right_points = &polygons[&right];
            let (Some(plane), Some(right_normal)) = (
                overlap_plane(left_points),
                polygon_normal3(right_points).map(canonical3),
            ) else {
                continue;
            };
            // 法線が平行で、かつ両面の全頂点が同じ平面に乗っているときだけ。
            if plane.normal.dot(right_normal).abs() < 1.0 - 1e-9 {
                continue;
            }
            let coincident = right_points
                .iter()
                .chain(left_points)
                .all(|point| plane.normal.dot(*point - plane.origin).abs() <= 1e-9);
            if !coincident {
                continue;
            }
            let left_2d = project2(left_points, &plane);
            let right_2d = project2(right_points, &plane);
            let left_triangles = triangulate_polygon(&left_2d);
            let right_triangles = triangulate_polygon(&right_2d);
            let Some((witness, _)) =
                overlap_witness(&left_2d, &left_triangles, &right_2d, &right_triangles)
            else {
                continue;
            };
            let shared_hinge = edges_of[&left]
                .intersection(&edges_of[&right])
                .copied()
                .find(|edge| diagram.hinges.iter().any(|(hinge, _)| hinge == edge));
            overlaps.push(CoincidentOverlap {
                left,
                right,
                normal: plane.normal,
                witness: plane.origin + plane.u * witness[0] + plane.v * witness[1],
                shared_hinge,
            });
        }
    }
    overlaps
}

/// 重なっている2面の、指定姿勢での高さの差(左 − 右)。決められないとき `None`。
fn probe_height_difference(
    exact: &BTreeMap<FaceId, Vec<V3>>,
    probe: &BTreeMap<FaceId, Vec<V3>>,
    overlap: &CoincidentOverlap,
) -> Option<f64> {
    let left = map_material_point(
        exact.get(&overlap.left)?,
        probe.get(&overlap.left)?,
        overlap.witness,
    )?;
    let right = map_material_point(
        exact.get(&overlap.right)?,
        probe.get(&overlap.right)?,
        overlap.witness,
    )?;
    Some(left.dot(overlap.normal) - right.dot(overlap.normal))
}

/// 紙が裂けたとみなす辺の離れ。製品側 `motion.rs` と同じ値。
const SEAM_TEAR_TOLERANCE: f64 = 1e-6;

/// 折り目1本だけを少し戻したときに、どちらが上へ出るかで決める判定。
///
/// 2面が1本の折り目でつながっているとき、その折り目の角度を少し戻すと、
/// 紙はすり抜けられないので必ず片方が上へ出る。他の折り目は一切動かさないので、
/// 経路の刻み方にも22点のcheckpointにも依存しない。
///
/// ただし折り目1本だけを戻すと紙が裂ける展開図もある。裂けた形は実際には
/// 作れないので、裂けの量が `1e-6` 以上になった探りは答えとして採らない。
fn local_hinge_truth(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
    exact: &BTreeMap<FaceId, Vec<V3>>,
    overlap: &CoincidentOverlap,
) -> Option<bool> {
    let hinge = overlap.shared_hinge?;
    let angle = angles.get(&hinge).copied()?;
    if angle == 0.0 {
        return None;
    }
    for relief in [1e-7_f64, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1.0, 5.0] {
        let relaxed = angle.abs() - relief;
        if relaxed <= 0.0 {
            break;
        }
        let mut probe_angles = angles.clone();
        probe_angles.insert(hinge, angle.signum() * relaxed);
        let probe_frame3d = to_frame3d(cp, faces, &propagate(cp, faces, &probe_angles));
        if ori3_rigid::max_seam_gap(cp, faces, &probe_frame3d) >= SEAM_TEAR_TOLERANCE {
            return None;
        }
        let probe = frame_polygons(&probe_frame3d);
        let Some(difference) = probe_height_difference(exact, &probe, overlap) else {
            continue;
        };
        if difference.abs() > 1e-12 {
            return Some(difference > 0.0);
        }
    }
    None
}

/// いまの姿勢から平らな状態へ向けて、全ての折り目を同じ割合で戻しながら
/// 実際に解いた姿勢の梯子。裂けた段は捨てるので、残るのは実際に作れる形だけ。
///
/// 製品側は「全ヒンジを共通の角度で頭打ちにする」経路を使う。ここは
/// 「全ヒンジを共通の割合で縮める」経路で、刻み方が違う独立の道である。
fn solved_unfold_ladder(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
) -> Vec<(f64, BTreeMap<FaceId, Vec<V3>>)> {
    const SCALES: [f64; 12] = [
        1.0 - 1e-9,
        1.0 - 1e-8,
        1.0 - 1e-7,
        1.0 - 1e-6,
        1.0 - 1e-5,
        1.0 - 1e-4,
        1.0 - 1e-3,
        0.99,
        0.95,
        0.9,
        0.8,
        0.5,
    ];
    let mut warm = angles.clone();
    let mut ladder = Vec::with_capacity(SCALES.len());
    for scale in SCALES {
        let mut hard = angles
            .iter()
            .map(|(&hinge, &angle)| Driver {
                hinge,
                target_angle_deg: angle * scale,
            })
            .collect::<Vec<_>>();
        hard.sort_unstable_by_key(|driver| driver.hinge);
        // 重なり順は刻印させない。独立検証がアプリの重なり順に触れないため。
        let motion = solve_motion(cp, faces, &hard, None, Some(&warm), false);
        let seam = ori3_rigid::max_seam_gap(cp, faces, &motion.result.frame);
        let torn = seam >= SEAM_TEAR_TOLERANCE;
        let finite = motion
            .result
            .frame
            .faces
            .iter()
            .flat_map(|face| &face.polygon)
            .flatten()
            .all(|coordinate| coordinate.is_finite());
        warm = motion.result.angles.clone();
        if !torn && finite {
            ladder.push((seam, frame_polygons(&motion.result.frame)));
        }
    }
    ladder
}

/// 梯子を「いまの姿勢に近い段」から順に見て、最初に離れた段の上下で決める。
///
/// 返すのは (上下, 高さの差, その段の裂けの量)。**高さの差がその段の裂けの量より
/// 小さい場合、その答えは信用できない。** 面が離れている量より、紙がちぎれている
/// 量のほうが大きいためである。呼出し元が比を見られるよう、判定は変えずに両方返す。
fn ladder_truth(
    exact: &BTreeMap<FaceId, Vec<V3>>,
    ladder: &[(f64, BTreeMap<FaceId, Vec<V3>>)],
    overlap: &CoincidentOverlap,
) -> Option<(bool, f64, f64)> {
    for (seam, probe) in ladder {
        let Some(difference) = probe_height_difference(exact, probe, overlap) else {
            continue;
        };
        if difference.abs() > 1e-12 {
            return Some((difference > 0.0, difference, *seam));
        }
    }
    None
}

#[derive(Default, Debug, Clone, Copy)]
struct TopFaceAudit {
    /// 完全に重なっている面対の数。
    overlaps: usize,
    /// 折り目1本を戻す判定で上下が決まった面対の数。
    local_decided: usize,
    /// 全体を同じ割合で戻して解き直す判定で上下が決まった面対の数。
    ladder_decided: usize,
    /// 2つの独立判定が食い違った面対の数。
    truth_disagreements: usize,
    /// 独立判定とアプリの重なり順が食い違った面対の数。
    rank_mismatches: usize,
    /// どちらの独立判定でも決められなかった面対の数。
    undecided: usize,
}

/// 完全に重なっている面対それぞれについて「上に来るべき面」を独立に求め、
/// アプリの `surface_rank` が選ぶ面と突き合わせる。
fn audit_top_faces(
    cp: &CreasePattern,
    faces: &[Face],
    diagram: &Diagram,
    frame: &Frame3D,
    angles: &HashMap<EdgeId, f64>,
    mut report: impl FnMut(&CoincidentOverlap, bool, bool, Option<(f64, f64)>),
) -> TopFaceAudit {
    let exact = frame_polygons(frame);
    let ranks = frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    let mut audit = TopFaceAudit::default();
    let overlaps = coincident_overlaps(diagram, frame);
    if overlaps.is_empty() {
        return audit;
    }
    let ladder = solved_unfold_ladder(cp, faces, angles);
    for overlap in overlaps {
        audit.overlaps += 1;
        let local = local_hinge_truth(cp, faces, angles, &exact, &overlap);
        let ladder_answer = ladder_truth(&exact, &ladder, &overlap);
        if let (Some(local), Some((ladder_above, _, _))) = (local, ladder_answer)
            && local != ladder_above
        {
            audit.truth_disagreements += 1;
        }
        if local.is_some() {
            audit.local_decided += 1;
        }
        if ladder_answer.is_some() {
            audit.ladder_decided += 1;
        }
        // 実際に解いた梯子を正とする。決められなかった面対だけ、裂けていない
        // 折り目1本の探りを使う。
        let Some(truth_left_above) = ladder_answer.map(|(above, _, _)| above).or(local) else {
            audit.undecided += 1;
            continue;
        };
        let rank_left_above = ranks[&overlap.left] > ranks[&overlap.right];
        if rank_left_above != truth_left_above {
            audit.rank_mismatches += 1;
            report(
                &overlap,
                truth_left_above,
                rank_left_above,
                ladder_answer.map(|(_, height, seam)| (height, seam)),
            );
        }
    }
    audit
}

/// 重なり順をどの幾何から決めたかの名前。番号順の経路が残っていれば
/// `face-index` として数えられる。
fn surface_order_source_label(source: Option<SurfaceOrderSource>) -> &'static str {
    match source {
        Some(SurfaceOrderSource::SolvedFlatPath) => "solved-flat-path",
        Some(SurfaceOrderSource::FoldFramePath) => "fold-frame-path",
        Some(SurfaceOrderSource::CurrentDepths) => "current-depths",
        Some(SurfaceOrderSource::PropagatedFlatPath) => "propagated-flat-path",
        Some(SurfaceOrderSource::NoOverlap) => "no-overlap",
        None => "not-stamped",
    }
}

/// 段階2の測定。240通りの姿勢について
/// (1) 重なり順をどの経路で決めたかの件数
/// (2) 「上に来るべき面」を独立に求めた結果との食い違いの数
/// を出す。合否は付けない(§10.7.7)。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage2_random_pose_sweep_paths_and_top_faces() {
    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSES: usize = 240;
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage2-sweep", cp.clone(), 1.0, 1.0);
    let poses = sweep_poses(&cp, &faces, SEED, POSES);
    let mut sources = BTreeMap::<&'static str, usize>::new();
    let mut total = TopFaceAudit::default();
    let mut poses_with_mismatch = 0_usize;
    let mut poses_with_overlap = 0_usize;
    for pose in &poses {
        *sources.entry(pose.source_label).or_default() += 1;
        let mut lines = Vec::new();
        let audit = audit_top_faces(
            &cp,
            &faces,
            &diagram,
            &pose.frame,
            &pose.angles,
            |overlap, truth_left_above, rank_left_above, ladder| {
                // 梯子の高さの差と、その段の裂けの量を並べる。裂けのほうが大きければ
                // 「離れている」という判定は紙のちぎれに埋もれており、信用できない。
                let evidence = match ladder {
                    Some((height, seam)) => format!(
                        " ladder_height={height:.3e} ladder_seam={seam:.3e} height_over_seam={:.3}",
                        height.abs() / seam.max(f64::MIN_POSITIVE)
                    ),
                    None => " ladder=none".to_string(),
                };
                lines.push(format!(
                    "{}<->{} hinge={:?} truth_left_above={truth_left_above} rank_left_above={rank_left_above}{evidence}",
                    overlap.left, overlap.right, overlap.shared_hinge,
                ));
            },
        );
        total.overlaps += audit.overlaps;
        total.local_decided += audit.local_decided;
        total.ladder_decided += audit.ladder_decided;
        total.truth_disagreements += audit.truth_disagreements;
        total.rank_mismatches += audit.rank_mismatches;
        total.undecided += audit.undecided;
        if audit.overlaps > 0 {
            poses_with_overlap += 1;
        }
        if audit.rank_mismatches > 0 {
            poses_with_mismatch += 1;
            println!(
                "STAGE2MISMATCH {} source={} overlaps={} mismatches={} pairs=[{}] angles={:?}",
                pose.label,
                pose.source_label,
                audit.overlaps,
                audit.rank_mismatches,
                lines.join(" | "),
                pose.angles
                    .iter()
                    .map(|(&hinge, &angle)| (hinge, angle))
                    .collect::<BTreeMap<_, _>>()
                    .iter()
                    .map(|(hinge, angle)| format!("{hinge}:{angle:.6}"))
                    .collect::<Vec<_>>(),
            );
        }
    }
    for (label, count) in &sources {
        println!("STAGE2SOURCE {label}={count}");
    }
    println!(
        "STAGE2 seed={SEED:#x} poses={POSES} poses_with_overlap={poses_with_overlap} overlaps={} local_decided={} ladder_decided={} truth_disagreements={} undecided={} rank_mismatches={} poses_with_mismatch={poses_with_mismatch}",
        total.overlaps,
        total.local_decided,
        total.ladder_decided,
        total.truth_disagreements,
        total.undecided,
        total.rank_mismatches,
    );
}

/// 段階2で1件だけ残った食い違い(pose163 の面7と面13)を、
/// 独立判定と製品側の経路の両方から詳しく見る。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage2_remaining_mismatch_detail() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage2-detail", cp.clone(), 1.0, 1.0);
    let poses = sweep_poses(&cp, &faces, 0x0123_4567_89AB_CDEF, 240);
    let wanted = std::env::var("ORI3_STAGE2_POSE").unwrap_or_else(|_| "pose163".to_string());
    let pose = poses
        .into_iter()
        .find(|pose| pose.label.starts_with(&wanted))
        .expect("the requested pose is in the fixed sweep");
    println!("DETAIL pose={} angles={:?}", pose.label, {
        let mut angles = pose.angles.iter().map(|(&e, &a)| (e, a)).collect::<Vec<_>>();
        angles.sort_by_key(|pair| pair.0);
        angles
    });
    let exact = frame_polygons(&pose.frame);
    let overlaps = coincident_overlaps(&diagram, &pose.frame);
    let ranks = pose
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    println!("DETAIL source={} overlaps={}", pose.source_label, overlaps.len());
    for overlap in &overlaps {
        let local = local_hinge_truth(&cp, &faces, &pose.angles, &exact, overlap);
        println!(
            "DETAIL pair {}<->{} hinge={:?} rank_left={} rank_right={} local_truth={local:?}",
            overlap.left,
            overlap.right,
            overlap.shared_hinge,
            ranks[&overlap.left],
            ranks[&overlap.right],
        );
        // 全体を同じ割合で戻す梯子を、段ごとに裂けの量つきで出す。
        let mut warm = pose.angles.clone();
        for scale in [
            1.0 - 1e-9,
            1.0 - 1e-8,
            1.0 - 1e-7,
            1.0 - 1e-6,
            1.0 - 1e-5,
            1.0 - 1e-4,
            1.0 - 1e-3,
            0.99,
            0.95,
            0.9,
        ] {
            let mut hard = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| Driver {
                    hinge,
                    target_angle_deg: angle * scale,
                })
                .collect::<Vec<_>>();
            hard.sort_unstable_by_key(|driver| driver.hinge);
            let motion = solve_motion(&cp, &faces, &hard, None, Some(&warm), false);
            warm = motion.result.angles.clone();
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &motion.result.frame);
            let probe = frame_polygons(&motion.result.frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   ladder scale={scale:.10} seam={seam:.3e} height_difference={difference:?}",
            );
        }
        // 製品側 canonical_motion_surface_order と同じ「共通の角度で頭打ちにして
        // 各点を解き直す」経路。
        let mut solved_warm: Option<HashMap<EdgeId, f64>> = None;
        for checkpoint in [
            9.0_f64, 90.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999,
        ] {
            let mut hard = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| Driver {
                    hinge,
                    target_angle_deg: angle.signum() * angle.abs().min(checkpoint),
                })
                .collect::<Vec<_>>();
            hard.sort_unstable_by_key(|driver| driver.hinge);
            let motion = solve_motion(&cp, &faces, &hard, None, solved_warm.as_ref(), false);
            solved_warm = Some(motion.result.angles.clone());
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &motion.result.frame);
            let probe = frame_polygons(&motion.result.frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   solved-clamp checkpoint={checkpoint} seam={seam:.3e} height_difference={difference:?}",
            );
        }
        // 製品側と同じ「共通の角度で頭打ちにする」経路を、伝播だけで再現する。
        for checkpoint in [90.0_f64, 171.0, 179.0, 179.9, 179.99, 179.999] {
            let clamped = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| {
                    (hinge, angle.signum() * angle.abs().min(checkpoint))
                })
                .collect::<HashMap<_, _>>();
            let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &clamped));
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &frame);
            let probe = frame_polygons(&frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   clamp checkpoint={checkpoint} seam={seam:.3e} height_difference={difference:?}",
            );
        }
    }
}
