//! Step 144: solve, persist, and verify the reflection-symmetric standing pose.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_layers::{replay, resolve_driver_edges};
use ori3_model::{
    Document, Driver, DriverLine, EdgeKind, FoldStep, Frame3D, TechniqueKind, VertexId,
};
use ori3_rigid::{max_seam_gap, solve_near_with_reflection_symmetry, three_point_support};

const EPS: f64 = 1e-8;

#[derive(Clone, Debug)]
pub struct DevilFinalMetrics {
    pub faces: usize,
    pub hinges: usize,
    pub horn_tips: usize,
    pub boundary_corners: usize,
    pub symmetry_error: f64,
    pub max_seam_gap: f64,
    pub z_span: f64,
    pub support_area: f64,
    pub centroid_height: f64,
}

fn hinge_edges(faces: &[Face]) -> Vec<u32> {
    let mut owners = BTreeMap::<u32, BTreeSet<u32>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().insert(face.id);
        }
    }
    owners
        .into_iter()
        .filter_map(|(edge, owners)| (owners.len() == 2).then_some(edge))
        .collect()
}

fn seed_drivers(document: &Document) -> Vec<Driver> {
    let q = 2.0 - std::f64::consts::SQRT_2;
    let lines = [
        DriverLine {
            a: [1.0, 1.0],
            b: [0.0, q],
            target_angle_deg: -60.0,
        },
        DriverLine {
            a: [1.0, 1.0],
            b: [q, 0.0],
            target_angle_deg: -60.0,
        },
    ];
    let mut drivers = BTreeMap::<u32, f64>::new();
    for line in &lines {
        let resolved = resolve_driver_edges(&document.cp, line);
        assert_eq!(
            resolved.len(),
            10,
            "a Devil horn line has ten exact fragments"
        );
        for edge in resolved {
            let previous = drivers.insert(edge, line.target_angle_deg);
            assert!(previous.is_none_or(|angle| angle == line.target_angle_deg));
        }
    }
    assert_eq!(
        drivers.len(),
        20,
        "the paired horn folds drive twenty fragments"
    );
    drivers
        .into_iter()
        .map(|(hinge, target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect()
}

fn persistent_driver_lines(document: &Document, angles: &HashMap<u32, f64>) -> Vec<DriverLine> {
    let positions: HashMap<_, _> = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect();
    let mut edges = document
        .cp
        .edges
        .iter()
        .filter(|edge| angles.contains_key(&edge.id))
        .collect::<Vec<_>>();
    edges.sort_unstable_by_key(|edge| edge.id);
    edges
        .into_iter()
        .map(|edge| DriverLine {
            a: positions[&edge.v0],
            b: positions[&edge.v1],
            target_angle_deg: angles[&edge.id],
        })
        .collect()
}

fn folded_vertices(faces: &[Face], frame: &Frame3D) -> HashMap<VertexId, DVec3> {
    let by_face: HashMap<_, _> = faces.iter().map(|face| (face.id, face)).collect();
    let mut positions = HashMap::<VertexId, DVec3>::new();
    for face3d in &frame.faces {
        let face = by_face[&face3d.face];
        assert_eq!(face.vertices.len(), face3d.polygon.len());
        for (&vertex, &point) in face.vertices.iter().zip(&face3d.polygon) {
            let point = DVec3::from(point);
            if let Some(previous) = positions.insert(vertex, point) {
                assert!(
                    (previous - point).length() < 1e-7,
                    "shared vertex {vertex} tore"
                );
            }
        }
    }
    positions
}

fn reflection_error(
    positions: &HashMap<VertexId, DVec3>,
    mirrors: &HashMap<VertexId, VertexId>,
) -> f64 {
    let (first, second) = mirrors
        .iter()
        .map(|(&vertex, &mirror)| (positions[&vertex], positions[&mirror]))
        .max_by(|(a0, b0), (a1, b1)| {
            (*a0 - *b0)
                .length_squared()
                .total_cmp(&(*a1 - *b1).length_squared())
        })
        .expect("reflection pairs");
    let normal = (first - second).normalize();
    let offset = normal.dot((first + second) * 0.5);
    mirrors
        .iter()
        .map(|(&vertex, &mirror)| {
            let point = positions[&vertex];
            let reflected = point - 2.0 * normal * (normal.dot(point) - offset);
            (reflected - positions[&mirror]).length()
        })
        .fold(0.0, f64::max)
}

fn z_span(frame: &Frame3D) -> f64 {
    let (minimum, maximum) = frame.faces.iter().flat_map(|face| &face.polygon).fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), point| (minimum.min(point[2]), maximum.max(point[2])),
    );
    maximum - minimum
}

fn vertex_at(document: &Document, point: [f64; 2]) -> VertexId {
    document
        .cp
        .vertices
        .iter()
        .find(|vertex| (DVec2::from(vertex.pos) - DVec2::from(point)).length() < EPS)
        .unwrap_or_else(|| panic!("missing exact Devil vertex {point:?}"))
        .id
}

fn boundary_corner_count(document: &Document) -> usize {
    let positions: HashMap<_, _> = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    let mut neighbors = HashMap::<VertexId, Vec<VertexId>>::new();
    for edge in &document.cp.edges {
        if edge.kind == EdgeKind::Border {
            neighbors.entry(edge.v0).or_default().push(edge.v1);
            neighbors.entry(edge.v1).or_default().push(edge.v0);
        }
    }
    neighbors
        .iter()
        .filter(|(vertex, adjacent)| {
            adjacent.len() == 2 && {
                let a = (positions[&adjacent[0]] - positions[vertex]).normalize();
                let b = (positions[&adjacent[1]] - positions[vertex]).normalize();
                a.perp_dot(b).abs() > 0.5
            }
        })
        .count()
}

/// Append the exact step-144 Pose and verify its persisted replay.
pub fn append_step_144(document: &mut Document) -> DevilFinalMetrics {
    assert!(
        document
            .cp
            .edges
            .iter()
            .all(|edge| edge.kind != EdgeKind::Aux),
        "steps 17-143 must activate every precrease before the final Pose"
    );
    let faces = extract_faces(&document.cp);
    assert_eq!(
        faces.len(),
        110,
        "the complete Devil crease pattern has 110 faces"
    );
    let hinges = hinge_edges(&faces);
    assert_eq!(
        hinges.len(),
        177,
        "all 177 Devil crease fragments are hinges"
    );

    let hard = seed_drivers(document);
    let uniform = hinges
        .iter()
        .copied()
        .map(|hinge| (hinge, -30.0))
        .collect::<HashMap<_, _>>();
    let symmetric = solve_near_with_reflection_symmetry(
        &document.cp,
        &faces,
        &hard,
        &uniform,
        Some(&uniform),
        [[0.0, 0.0], [1.0, 1.0]],
    )
    .expect("the full Devil CP has a reflection-symmetric rigid pose");
    let solved = symmetric.result;
    assert!(solved.converged);
    assert!(solved.frame.warnings.is_empty());
    assert_eq!(solved.angles.len(), hinges.len());

    let drivers = persistent_driver_lines(document, &solved.angles);
    assert_eq!(drivers.len(), hinges.len());
    let id = u32::try_from(document.sequence.len()).expect("step ID fits u32");
    document.sequence.push(FoldStep {
        id,
        kind: TechniqueKind::Pose,
        drivers,
        layer_order: None,
        alignment: None,
        note: "手順144: 左右の腕を整え、両足と尾の三点で自立させる".to_string(),
    });

    let replayed = replay(document, document.sequence.len(), 1.0);
    assert!(
        replayed.warnings.is_empty(),
        "replay warnings: {:?}",
        replayed.warnings
    );
    assert!(
        replayed.skipped.is_empty(),
        "skipped: {:?}",
        replayed.skipped
    );
    assert!(
        replayed.frame.warnings.is_empty(),
        "frame warnings: {:?}",
        replayed.frame.warnings
    );
    assert_eq!(
        replayed.frame.faces.len(),
        faces.len(),
        "no final face is lost"
    );
    let solved_mirrored = solved
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.mirrored))
        .collect::<BTreeMap<_, _>>();
    let replayed_mirrored = replayed
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.mirrored))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        replayed_mirrored, solved_mirrored,
        "step 144の保存・再生で面の鏡映偶奇を変えない"
    );
    assert_eq!(
        solved_mirrored.len(),
        faces.len(),
        "全完成面の鏡映偶奇を保存する"
    );
    let gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
    assert!(gap < 1e-6, "step 144 max_seam_gap={gap:.3e}");

    let positions = folded_vertices(&faces, &replayed.frame);
    let symmetry_error = reflection_error(&positions, &symmetric.mirrored_vertices);
    assert!(
        symmetry_error < 1e-9,
        "left/right error={symmetry_error:.3e}"
    );
    let span = z_span(&replayed.frame);
    assert!(span > 0.1, "the completed Devil must be three-dimensional");

    let q = 2.0 - std::f64::consts::SQRT_2;
    let horn_tips = [vertex_at(document, [0.0, q]), vertex_at(document, [q, 0.0])];
    assert_ne!(horn_tips[0], horn_tips[1]);
    assert_eq!(symmetric.mirrored_vertices[&horn_tips[0]], horn_tips[1]);
    let boundary_corners = boundary_corner_count(document);
    assert_eq!(
        boundary_corners, 4,
        "the source square retains four boundary corners"
    );

    let support = three_point_support(&faces, &replayed.frame, [1, 2, 3])
        .expect("feet and tail define a support plane");
    assert!(support.one_sided, "no vertex may pass below the floor");
    assert!(
        support.centroid_projection_inside,
        "centroid must lie in the support triangle"
    );
    assert!(
        support.stable,
        "the completed Devil must stand on three points"
    );

    DevilFinalMetrics {
        faces: faces.len(),
        hinges: hinges.len(),
        horn_tips: horn_tips.len(),
        boundary_corners,
        symmetry_error,
        max_seam_gap: gap,
        z_span: span,
        support_area: support.support_area,
        centroid_height: support.centroid_height,
    }
}
