#[path = "../src/pose_oracle.rs"]
mod pose_oracle;

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_model::{CreasePattern, Document, Driver, EdgeKind, Face3D, Frame3D, Paper, Vertex};
use ori3_rigid::solve;

use pose_oracle::{
    PoseDepthExpectation, PoseDifference, PoseExpectation, PoseLandmarkExpectation, Ray3,
    evaluate_pose, raycast_faces,
};

fn folded_pose() -> (CreasePattern, Vec<Face>, Frame3D, u32, [[f64; 2]; 2]) {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let inserted = insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
    assert_eq!(inserted.len(), 1);
    let hinge = inserted[0];
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 2);
    let solved = solve(
        &document.cp,
        &faces,
        &[Driver {
            hinge,
            target_angle_deg: 60.0,
        }],
        None,
    );
    assert!(solved.converged, "warnings={:?}", solved.frame.warnings);

    let material_positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<std::collections::HashMap<_, _>>();
    let rigid_edge = [
        material_positions[&faces[0].vertices[0]],
        material_positions[&faces[0].vertices[1]],
    ];
    (document.cp, faces, solved.frame, hinge, rigid_edge)
}

fn folded_expectation(faces: &[Face], rigid_edge: [[f64; 2]; 2]) -> PoseExpectation {
    let expected_distance = DVec2::from(rigid_edge[0]).distance(DVec2::from(rigid_edge[1]));
    let mut expectation = PoseExpectation::from_faces(faces);
    expectation.min_z_span = 0.1;
    expectation.landmarks = vec![PoseLandmarkExpectation::Distance {
        first_material: rigid_edge[0],
        second_material: rigid_edge[1],
        expected_distance,
    }];
    expectation
}

#[test]
fn accepts_a_complete_watertight_non_flat_pose() {
    let (cp, faces, frame, _, rigid_edge) = folded_pose();
    let expectation = folded_expectation(&faces, rigid_edge);

    let report = evaluate_pose(&cp, &faces, &frame, &expectation);

    assert!(report.is_match(), "{}", report.explanations().join("\n"));
    assert!(report.actual.max_seam_gap < 1.0e-6);
    assert!(report.actual.z_span >= 0.1);
    assert_eq!(report.landmark_samples.len(), 1);
}

#[test]
fn reports_a_missing_face() {
    let (cp, faces, mut frame, _, rigid_edge) = folded_pose();
    let missing = frame.faces.pop().expect("two-face pose").face;
    let expectation = folded_expectation(&faces, rigid_edge);

    let report = evaluate_pose(&cp, &faces, &frame, &expectation);

    assert!(!report.is_match());
    assert!(
        report
            .differences
            .iter()
            .any(|difference| matches!(difference, PoseDifference::MissingFace { face } if *face == missing)),
        "differences={:?}",
        report.differences
    );
}

#[test]
fn reports_a_torn_shared_vertex_and_seam() {
    let (cp, faces, mut frame, hinge, rigid_edge) = folded_pose();
    let edge = cp
        .edges
        .iter()
        .find(|edge| edge.id == hinge)
        .expect("inserted hinge");
    let owners = faces
        .iter()
        .filter(|face| face.edges.contains(&hinge))
        .collect::<Vec<_>>();
    assert_eq!(owners.len(), 2);
    let torn_face = owners[1];
    let vertex_index = torn_face
        .vertices
        .iter()
        .position(|vertex| *vertex == edge.v0)
        .expect("hinge endpoint belongs to both faces");
    frame
        .faces
        .iter_mut()
        .find(|face| face.face == torn_face.id)
        .expect("frame contains hinge owner")
        .polygon[vertex_index][2] += 0.01;
    let expectation = folded_expectation(&faces, rigid_edge);

    let report = evaluate_pose(&cp, &faces, &frame, &expectation);

    assert!(report.differences.iter().any(|difference| matches!(
        difference,
        PoseDifference::SharedVertexMismatch { vertex, .. } if *vertex == edge.v0
    )));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| matches!(difference, PoseDifference::SeamGap { .. })),
        "differences={:?}",
        report.differences
    );
}

fn stacked_faces() -> (CreasePattern, Vec<Face>, Frame3D) {
    let vertices = vec![
        Vertex {
            id: 0,
            pos: [0.0, 0.0],
        },
        Vertex {
            id: 1,
            pos: [1.0, 0.0],
        },
        Vertex {
            id: 2,
            pos: [1.0, 1.0],
        },
        Vertex {
            id: 3,
            pos: [0.0, 1.0],
        },
        Vertex {
            id: 4,
            pos: [2.0, 0.0],
        },
        Vertex {
            id: 5,
            pos: [3.0, 0.0],
        },
        Vertex {
            id: 6,
            pos: [3.0, 1.0],
        },
        Vertex {
            id: 7,
            pos: [2.0, 1.0],
        },
    ];
    let cp = CreasePattern {
        vertices,
        edges: Vec::new(),
        next_vertex_id: 8,
        next_edge_id: 0,
    };
    let faces = vec![
        Face {
            id: 0,
            vertices: vec![0, 1, 2, 3],
            edges: Vec::new(),
        },
        Face {
            id: 1,
            vertices: vec![4, 5, 6, 7],
            edges: Vec::new(),
        },
    ];
    let square = |z| vec![[0.0, 0.0, z], [1.0, 0.0, z], [1.0, 1.0, z], [0.0, 1.0, z]];
    let frame = Frame3D {
        faces: vec![
            Face3D {
                face: 0,
                polygon: square(0.0),
                layer: 0,
                surface_rank: 0,
                mirrored: false,
            },
            Face3D {
                face: 1,
                polygon: square(1.0),
                layer: 0,
                surface_rank: 0,
                mirrored: false,
            },
        ],
        warnings: Vec::new(),
    };
    (cp, faces, frame)
}

#[test]
fn ray_depth_uses_3d_distance_and_reports_wrong_order() {
    let (cp, faces, frame) = stacked_faces();
    let ray = Ray3 {
        origin: [0.5, 0.5, 2.0],
        direction: [0.0, 0.0, -1.0],
    };
    let hits = raycast_faces(&frame, ray, 1.0e-8).expect("valid ray");
    assert_eq!(hits.iter().map(|hit| hit.face).collect::<Vec<_>>(), [1, 0]);
    assert!(hits[0].distance < hits[1].distance);

    let mut expectation = PoseExpectation::from_faces(&faces);
    expectation.min_z_span = 0.5;
    expectation.landmarks = vec![PoseLandmarkExpectation::Position {
        material: [0.0, 0.0],
        expected: [0.0, 0.0, 0.0],
    }];
    expectation.depth_probes = vec![PoseDepthExpectation {
        ray,
        expected_near_to_far: vec![0, 1],
    }];

    let report = evaluate_pose(&cp, &faces, &frame, &expectation);

    assert_eq!(report.depth_samples[0].hits, hits);
    assert!(report.differences.iter().any(|difference| matches!(
        difference,
        PoseDifference::DepthOrder {
            expected_near_to_far,
            actual_near_to_far,
            ..
        } if expected_near_to_far == &[0, 1] && actual_near_to_far == &[1, 0]
    )));
}
