use glam::DVec3;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_layers::{PoseStepInput, apply_pose_step};
use ori3_model::{Document, DriverLine, EdgeId, EdgeKind, FoldStep, Frame3D, Paper, TechniqueKind};

const FIRST: [[f64; 2]; 2] = [[1.0 / 3.0, 0.0], [1.0 / 3.0, 1.0]];
const SECOND: [[f64; 2]; 2] = [[2.0 / 3.0, 0.0], [2.0 / 3.0, 1.0]];

fn driver(line: [[f64; 2]; 2], angle: f64) -> DriverLine {
    DriverLine {
        a: line[0],
        b: line[1],
        target_angle_deg: angle,
    }
}

fn two_hinge_document() -> (Document, EdgeId, EdgeId) {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let first = insert_segment(&mut document.cp, FIRST[0], FIRST[1], EdgeKind::Mountain);
    let second = insert_segment(&mut document.cp, SECOND[0], SECOND[1], EdgeKind::Valley);
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    (document, first[0], second[0])
}

fn face_normal(frame: &Frame3D, face: u32) -> DVec3 {
    let polygon = &frame
        .faces
        .iter()
        .find(|candidate| candidate.face == face)
        .expect("face remains in the frame")
        .polygon;
    assert!(polygon.len() >= 3);
    let origin = DVec3::from(polygon[0]);
    let normal = (DVec3::from(polygon[1]) - origin)
        .cross(DVec3::from(polygon[2]) - origin)
        .normalize();
    assert!(normal.is_finite());
    normal
}

fn hinge_angle(faces: &[Face], frame: &Frame3D, hinge: EdgeId) -> f64 {
    let adjacent = faces
        .iter()
        .filter(|face| face.edges.contains(&hinge))
        .map(|face| face.id)
        .collect::<Vec<_>>();
    assert_eq!(adjacent.len(), 2, "the test crease is a two-face hinge");
    let first = face_normal(frame, adjacent[0]);
    let second = face_normal(frame, adjacent[1]);
    first.dot(second).clamp(-1.0, 1.0).acos().to_degrees()
}

#[test]
fn successive_pose_steps_update_only_named_hinges_and_keep_prior_angles() {
    let (mut document, first_hinge, second_hinge) = two_hinge_document();
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 3);

    // A prior ordinary flat-fold step remains part of the cumulative command
    // map when the first Pose step updates only the other hinge.
    document.sequence.push(FoldStep {
        id: 0,
        kind: TechniqueKind::Simple,
        drivers: vec![driver(SECOND, 180.0)],
        layer_order: None,
        alignment: None,
        curved_inside_reverse: None,
        finish_soft: None,
        note: "pre-existing flat fold".to_string(),
    });

    let first = apply_pose_step(
        &mut document,
        PoseStepInput {
            driver_updates: vec![driver(FIRST, 35.0)],
            note: "raise the first panel".to_string(),
        },
    )
    .expect("first pose update is valid");
    assert_eq!(first.step_id, 1);
    assert!(first.max_seam_gap < 1e-6);
    assert_eq!(first.frame.faces.len(), faces.len());
    assert!((hinge_angle(&faces, &first.frame, first_hinge) - 35.0).abs() < 1e-5);
    assert!((hinge_angle(&faces, &first.frame, second_hinge) - 180.0).abs() < 1e-5);

    let second = apply_pose_step(
        &mut document,
        PoseStepInput {
            driver_updates: vec![driver(SECOND, -40.0)],
            note: "bend the second panel".to_string(),
        },
    )
    .expect("second pose update is valid");
    assert_eq!(second.step_id, 2);
    assert!(second.max_seam_gap < 1e-6);
    assert_eq!(second.frame.faces.len(), faces.len());
    assert!((hinge_angle(&faces, &second.frame, first_hinge) - 35.0).abs() < 1e-5);
    assert!((hinge_angle(&faces, &second.frame, second_hinge) - 40.0).abs() < 1e-5);

    assert_eq!(document.sequence.len(), 3);
    assert_eq!(document.sequence[1].kind, TechniqueKind::Pose);
    assert_eq!(document.sequence[1].drivers, vec![driver(FIRST, 35.0)]);
    assert_eq!(document.sequence[1].note, "raise the first panel");
    assert_eq!(document.sequence[2].kind, TechniqueKind::Pose);
    assert_eq!(document.sequence[2].drivers, vec![driver(SECOND, -40.0)]);
    assert_eq!(document.sequence[2].note, "bend the second panel");
}

#[test]
fn invalid_unresolved_nonfinite_and_conflicting_updates_roll_back() {
    let (mut document, _, _) = two_hinge_document();

    let before = document.clone();
    let error = apply_pose_step(
        &mut document,
        PoseStepInput {
            driver_updates: vec![driver([[0.0, 0.5], [1.0, 0.5]], 30.0)],
            note: String::new(),
        },
    )
    .expect_err("an unresolved logical line is rejected");
    assert!(error.contains("does not resolve"), "{error}");
    assert_eq!(document, before);

    let error = apply_pose_step(
        &mut document,
        PoseStepInput {
            driver_updates: vec![driver(FIRST, f64::NAN)],
            note: String::new(),
        },
    )
    .expect_err("a non-finite target angle is rejected");
    assert!(error.contains("non-finite"), "{error}");
    assert_eq!(document, before);

    let error = apply_pose_step(
        &mut document,
        PoseStepInput {
            driver_updates: vec![driver(FIRST, 20.0), driver(FIRST, 25.0)],
            note: String::new(),
        },
    )
    .expect_err("conflicting duplicate hinge updates are rejected");
    assert!(error.contains("conflicting"), "{error}");
    assert_eq!(document, before);
}
