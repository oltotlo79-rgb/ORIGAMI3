use std::collections::HashSet;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::{
    FlatMotionInput, FlatState, FoldDirection, FoldThroughInput, MotionPart,
    compose_flat_motion_step, flat_state_at, fold_through, replay,
};
use ori3_model::{Document, DriverLine, FoldStep, Paper, TechniqueKind};

fn square() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn append(document: &mut Document, mut step: FoldStep) {
    step.id = u32::try_from(document.sequence.len()).unwrap();
    document.sequence.push(step);
}

fn state(document: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&document.cp);
    let (state, warnings) = flat_state_at(document, &faces, document.sequence.len()).unwrap();
    assert!(warnings.is_empty(), "flat-state warnings: {warnings:?}");
    (faces, state)
}

fn apply_fold(document: &mut Document, input: &FoldThroughInput) {
    let (faces, before) = state(document);
    let mut cp = document.cp.clone();
    let result = fold_through(&mut cp, &faces, &before, input).unwrap();
    assert!(result.warnings.is_empty());
    document.cp = cp;
    append(document, result.step);
}

fn assert_same_state(actual: &FlatState, expected: &FlatState, faces: &[Face]) {
    assert_eq!(actual.order, expected.order);
    for face in faces {
        assert!(
            actual.placements[&face.id].approx_eq(&expected.placements[&face.id], 1e-7),
            "face {} differs: {:?} != {:?}",
            face.id,
            actual.placements[&face.id],
            expected.placements[&face.id]
        );
    }
}

fn folded_bbox(
    cp: &ori3_model::CreasePattern,
    faces: &[Face],
    state: &FlatState,
) -> ([f64; 2], [f64; 2]) {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<std::collections::HashMap<_, _>>();
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    for face in faces {
        for vertex in &face.vertices {
            let point = state.placements[&face.id].apply(positions[vertex]);
            lo = lo.min(point);
            hi = hi.max(point);
        }
    }
    ([lo.x, lo.y], [hi.x, hi.y])
}

#[test]
fn validated_operations_compose_to_one_replayable_simple_step() {
    let vertical = FoldThroughInput {
        line: [[0.5, 0.0], [0.5, 1.0]],
        keep_side_point: [0.0, 0.5],
        target_layers: None,
        direction: FoldDirection::Up,
    };
    let horizontal = FoldThroughInput {
        line: [[0.0, 0.5], [0.5, 0.5]],
        keep_side_point: [0.25, 0.0],
        target_layers: None,
        direction: FoldDirection::Up,
    };

    let mut expanded = square();
    apply_fold(&mut expanded, &vertical);
    apply_fold(&mut expanded, &horizontal);
    let (expected_faces, expected_state) = state(&expanded);
    assert_eq!(expanded.sequence.len(), 2);

    let mut compacted = square();
    let mut result = compose_flat_motion_step(&mut compacted, |session| {
        session.apply_fold_through(&vertical)?;
        session.apply_fold_through(&horizontal)?;
        assert_eq!(session.applied_steps(), 2);
        Ok(())
    })
    .unwrap();

    assert_eq!(
        compacted.sequence.len(),
        0,
        "the caller appends the returned step"
    );
    assert_eq!(compacted.cp, expanded.cp, "all CP creases are retained");
    assert_eq!(result.step.kind, TechniqueKind::Simple);
    assert!(!result.added_edges.is_empty());
    assert!(
        result
            .added_edges
            .iter()
            .all(|id| compacted.cp.edges.iter().any(|edge| edge.id == *id))
    );
    assert_same_state(&result.state, &expected_state, &expected_faces);

    result.step.id = 0;
    compacted.sequence.push(result.step);
    assert_eq!(compacted.sequence.len(), 1);
    let (actual_faces, actual_state) = state(&compacted);
    assert_eq!(actual_faces.len(), expected_faces.len());
    assert_same_state(&actual_state, &expected_state, &actual_faces);

    let replayed = replay(&compacted, 1, 1.0);
    assert!(replayed.skipped.is_empty());
    assert!(replayed.warnings.is_empty());
    assert!(replayed.frame.warnings.is_empty());
    let replayed_faces = replayed
        .frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<HashSet<_>>();
    assert_eq!(
        replayed_faces,
        actual_faces.iter().map(|face| face.id).collect()
    );
}

#[test]
fn skipped_recorded_step_is_rejected_atomically() {
    let mut document = square();
    let before = document.clone();
    let bogus = FoldStep {
        id: 99,
        kind: TechniqueKind::Simple,
        drivers: vec![DriverLine {
            a: [0.25, 0.0],
            b: [0.25, 1.0],
            target_angle_deg: 180.0,
        }],
        layer_order: None,
        alignment: None,
        curved_inside_reverse: None,
        finish_soft: None,
        note: String::new(),
    };

    let error = compose_flat_motion_step(&mut document, |session| {
        session.apply_fold_step(bogus)?;
        Ok(())
    })
    .unwrap_err();
    assert!(
        error.contains("warning") || error.contains("skipped"),
        "unexpected error: {error}"
    );
    assert_eq!(document, before, "failed composition must be atomic");
}

#[test]
fn warning_from_an_elementary_motion_is_rejected_atomically() {
    let mut document = square();
    apply_fold(
        &mut document,
        &FoldThroughInput {
            line: [[0.75, 0.0], [0.75, 1.0]],
            keep_side_point: [0.5, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    );
    apply_fold(
        &mut document,
        &FoldThroughInput {
            line: [[1.0, 0.0], [1.0, 1.0]],
            keep_side_point: [0.9, 0.5],
            target_layers: None,
            direction: FoldDirection::Down,
        },
    );
    let before = document.clone();

    let error = compose_flat_motion_step(&mut document, |session| {
        let state = session.state().clone();
        let faces = session.faces().to_vec();
        let (lo, hi) = folded_bbox(session.crease_pattern(), &faces, &state);
        let cut = 0.5 * (lo[1] + hi[1]);
        let line = [[lo[0], cut], [hi[0], cut]];
        let upper = [0.5 * (lo[0] + hi[0]), hi[1] - 0.1 * (hi[1] - lo[1])];
        let targets = vec![state.order[1], state.order[2]];
        session.apply_flat_motion(&FlatMotionInput {
            parts: vec![MotionPart::fold(targets, line, upper, FoldDirection::Up)],
            kind: TechniqueKind::Simple,
        })?;
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("warnings"), "unexpected error: {error}");
    assert_eq!(document, before, "warning rejection must be atomic");
}
