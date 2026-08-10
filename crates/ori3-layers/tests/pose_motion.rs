//! High-level non-flat Pose solving on a four-face closed loop.

use std::collections::HashMap;

use ori3_model::{CreasePattern, Document, Edge, EdgeKind, Paper, TechniqueKind, Vertex};

// `pose_motion` is deliberately tested before it is wired into `lib.rs`.
// Re-export its existing crate-level dependencies so the production source can
// be compiled here without maintaining a test-only copy of the implementation.
pub use ori3_layers::{PoseStepInput, ReplayResult, apply_pose_step, replay};

#[path = "../src/pose_motion.rs"]
mod pose_motion;

use pose_motion::{
    PoseAngleTarget, PoseEdgeActivation, PoseMotionInput, solve_and_apply_pose_step,
};

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

/// Four sectors around one internal vertex. The sector angles
/// 50/60/130/120 degrees satisfy Kawasaki's theorem. Activating the four Aux
/// rays creates a four-face adjacency loop with one closure constraint.
fn closed_loop_document() -> Document {
    let p1x = 0.5 + 0.5 * 50f64.to_radians().cos() / 50f64.to_radians().sin();
    let p2x = 0.5 + 0.5 * 110f64.to_radians().cos() / 110f64.to_radians().sin();
    let p3x = 0.5 + 0.5 / 240f64.to_radians().sin().abs() * 240f64.to_radians().cos();
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    document.cp = CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, p3x, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 0.5),
            vertex(4, 1.0, 1.0),
            vertex(5, p1x, 1.0),
            vertex(6, p2x, 1.0),
            vertex(7, 0.0, 1.0),
            vertex(8, 0.5, 0.5),
        ],
        edges: vec![
            edge(0, 0, 1, EdgeKind::Border),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 3, EdgeKind::Border),
            edge(3, 3, 4, EdgeKind::Border),
            edge(4, 4, 5, EdgeKind::Border),
            edge(5, 5, 6, EdgeKind::Border),
            edge(6, 6, 7, EdgeKind::Border),
            edge(7, 7, 0, EdgeKind::Border),
            edge(8, 8, 3, EdgeKind::Aux),
            edge(9, 8, 5, EdgeKind::Aux),
            edge(10, 8, 6, EdgeKind::Aux),
            edge(11, 8, 1, EdgeKind::Aux),
        ],
        next_vertex_id: 9,
        next_edge_id: 12,
    };
    document
}

fn activations() -> Vec<PoseEdgeActivation> {
    vec![
        PoseEdgeActivation {
            edge_id: 8,
            kind: EdgeKind::Mountain,
        },
        PoseEdgeActivation {
            edge_id: 9,
            kind: EdgeKind::Valley,
        },
        PoseEdgeActivation {
            edge_id: 10,
            kind: EdgeKind::Mountain,
        },
        PoseEdgeActivation {
            edge_id: 11,
            kind: EdgeKind::Mountain,
        },
    ]
}

fn target(edge_id: u32, target_angle_deg: f64) -> PoseAngleTarget {
    PoseAngleTarget {
        edge_id,
        target_angle_deg,
    }
}

fn assert_angle_maps_close(expected: &HashMap<u32, f64>, actual: &HashMap<u32, f64>) {
    assert_eq!(actual.len(), expected.len());
    for (&edge, &angle) in expected {
        assert!(
            (actual[&edge] - angle).abs() < 1e-9,
            "edge {edge}: expected {angle}, got {}",
            actual[&edge]
        );
    }
}

#[test]
fn branch_hints_select_between_distinct_closed_modes() {
    let solve_branch = |hints: Vec<PoseAngleTarget>| {
        let mut document = closed_loop_document();
        solve_and_apply_pose_step(
            &mut document,
            PoseMotionInput {
                activations: activations(),
                drivers: vec![target(8, 90.0)],
                branch_hints: hints,
                note: String::new(),
            },
        )
        .expect("both seeded kinematic modes close")
        .hinge_angles
    };

    let mountain_valley_mode =
        solve_branch(vec![target(9, -60.0), target(10, 60.0), target(11, 60.0)]);
    let alternate_mode = solve_branch(vec![
        target(9, -165.0),
        target(10, -90.0),
        target(11, -165.0),
    ]);

    assert!(mountain_valley_mode[&9] < -1.0);
    assert!(alternate_mode[&9] < -120.0);
    assert!(mountain_valley_mode[&10] > 1.0);
    assert!(alternate_mode[&10] < -1.0);
    assert!(mountain_valley_mode[&11] > 1.0);
    assert!(alternate_mode[&11] < -120.0);
}

#[test]
fn branch_seed_derives_and_persists_every_closed_loop_angle_across_pose_steps() {
    let mut document = closed_loop_document();
    let first = solve_and_apply_pose_step(
        &mut document,
        PoseMotionInput {
            activations: activations(),
            drivers: vec![target(8, 90.0)],
            branch_hints: vec![target(9, -60.0), target(10, 60.0), target(11, 60.0)],
            note: "raise the four-sector vertex".to_string(),
        },
    )
    .expect("the branch seed selects a closed non-flat pose");

    assert_eq!(first.step_id, 0);
    assert!(first.max_seam_gap < 1e-6);
    assert!(first.frame.warnings.is_empty());
    assert_eq!(first.frame.faces.len(), 4);
    assert_eq!(first.hinge_angles.len(), 4);
    assert!((first.hinge_angles[&8] - 90.0).abs() < 1e-9);
    for dependent in [9, 10, 11] {
        assert!(
            first.hinge_angles[&dependent].abs() > 1.0,
            "dependent hinge {dependent} stayed flat: {:?}",
            first.hinge_angles
        );
    }
    assert!(
        document.cp.edges[8..]
            .iter()
            .all(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
    );
    assert_eq!(document.sequence.len(), 1);
    assert_eq!(document.sequence[0].kind, TechniqueKind::Pose);
    assert_eq!(document.sequence[0].drivers.len(), 4);
    assert_eq!(document.sequence[0].note, "raise the four-sector vertex");

    // DriverLine persistence uses every exact CP edge, in edge-ID order, and
    // keeps the solver's f64 angles without rounding.
    for ((edge_id, crease), line) in (8u32..=11)
        .zip(document.cp.edges[8..].iter())
        .zip(&document.sequence[0].drivers)
    {
        assert_eq!(crease.id, edge_id);
        let a = document.cp.vertices[crease.v0 as usize].pos;
        let b = document.cp.vertices[crease.v1 as usize].pos;
        assert_eq!(line.a, a);
        assert_eq!(line.b, b);
        assert_eq!(line.target_angle_deg, first.hinge_angles[&edge_id]);
    }

    let replayed = replay(&document, document.sequence.len(), 1.0);
    assert!(replayed.skipped.is_empty());
    assert!(replayed.warnings.is_empty());
    assert!(replayed.frame.warnings.is_empty());
    assert_angle_maps_close(&first.hinge_angles, &replayed.hinge_angles);

    // A second solve starts from the persisted non-flat Pose, rather than from
    // a flat branch, and appends another complete Pose solution.
    let second_hints = first
        .hinge_angles
        .iter()
        .filter(|(edge, _)| **edge != 8)
        .map(|(&edge_id, &target_angle_deg)| PoseAngleTarget {
            edge_id,
            target_angle_deg,
        })
        .collect();
    let second = solve_and_apply_pose_step(
        &mut document,
        PoseMotionInput {
            activations: Vec::new(),
            drivers: vec![target(8, 110.0)],
            branch_hints: second_hints,
            note: "continue from the first pose".to_string(),
        },
    )
    .expect("a persisted pose is a valid start for another pose");
    assert_eq!(second.step_id, 1);
    assert!((second.hinge_angles[&8] - 110.0).abs() < 1e-9);
    assert!(second.max_seam_gap < 1e-6);
    assert_eq!(document.sequence.len(), 2);
    assert_eq!(document.sequence[1].drivers.len(), 4);
    let replayed = replay(&document, document.sequence.len(), 1.0);
    assert_angle_maps_close(&second.hinge_angles, &replayed.hinge_angles);
}

#[test]
fn invalid_activation_and_nonconvergent_closed_loop_roll_back_every_change() {
    let mut document = closed_loop_document();
    let before = document.clone();

    let error = solve_and_apply_pose_step(
        &mut document,
        PoseMotionInput {
            activations: vec![activations()[0], activations()[0]],
            drivers: vec![target(8, 45.0)],
            branch_hints: Vec::new(),
            note: String::new(),
        },
    )
    .expect_err("duplicate activation is invalid");
    assert!(error.contains("duplicates edge 8"), "{error}");
    assert_eq!(document, before);

    // At edge 8 = 180 degrees this vertex can only close with the other
    // creases also fully folded. Fixing edge 9 at -90 is contradictory, so the
    // rigid solve must fail after activation without leaking any mutation.
    let error = solve_and_apply_pose_step(
        &mut document,
        PoseMotionInput {
            activations: activations(),
            drivers: vec![target(8, 180.0), target(9, -90.0)],
            branch_hints: Vec::new(),
            note: "must not be persisted".to_string(),
        },
    )
    .expect_err("contradictory hard drivers cannot close the face loop");
    assert!(error.contains("did not converge"), "{error}");
    assert_eq!(document, before);
}

#[test]
fn invalid_driver_and_hint_inputs_are_explicit_and_transactional() {
    let mut document = closed_loop_document();
    let before = document.clone();

    let cases = [
        (Vec::new(), Vec::new(), "at least one hard driver"),
        (vec![target(8, 181.0)], Vec::new(), "outside [-180, 180]"),
        (
            vec![target(8, 45.0)],
            vec![target(9, f64::NAN)],
            "non-finite",
        ),
    ];
    for (drivers, branch_hints, expected) in cases {
        let error = solve_and_apply_pose_step(
            &mut document,
            PoseMotionInput {
                activations: activations(),
                drivers,
                branch_hints,
                note: String::new(),
            },
        )
        .expect_err("invalid input must be rejected");
        assert!(error.contains(expected), "{error}");
        assert_eq!(document, before);
    }
}
