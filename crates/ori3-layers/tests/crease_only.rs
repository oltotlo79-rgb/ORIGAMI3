use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::{
    CreaseOnlyInput, FlatState, FoldDirection, FoldThroughInput, ReverseOpenCreaseInput,
    crease_only, flat_state_at, fold_through, point_in_face, replay, representative_point,
    reverse_open_crease_sense,
};
use ori3_model::{CreasePattern, Document, EdgeKind, FoldStep, Paper};
use ori3_rigid::max_seam_gap;

fn append_step(document: &mut Document, mut step: FoldStep) {
    step.id = u32::try_from(document.sequence.len()).expect("step ID fits in u32");
    document.sequence.push(step);
}

fn paper_area(cp: &CreasePattern, faces: &[Face]) -> f64 {
    let positions: HashMap<_, _> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    faces
        .iter()
        .map(|face| {
            let polygon: Vec<_> = face.vertices.iter().map(|id| positions[id]).collect();
            (0..polygon.len())
                .map(|i| polygon[i].perp_dot(polygon[(i + 1) % polygon.len()]))
                .sum::<f64>()
                .abs()
                * 0.5
        })
        .sum()
}

#[test]
fn selected_flap_can_be_creased_and_unfolded_without_moving_or_tearing() {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });

    // Make two coincident layers, so target_layers is tested rather than merely
    // accepted on a one-face sheet.
    let initial_faces = extract_faces(&document.cp);
    let initial_state = FlatState::initial(&document.cp, &initial_faces);
    let first = fold_through(
        &mut document.cp,
        &initial_faces,
        &initial_state,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("fold the right half onto the left half");
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    append_step(&mut document, first.step);

    let before_cp = document.cp.clone();
    let before_faces = extract_faces(&before_cp);
    let before_state = first.state;
    assert_eq!(
        before_faces.len(),
        2,
        "the preliminary fold makes two flaps"
    );
    let selected = before_faces
        .iter()
        .find(|face| before_state.placements[&face.id].mirrored)
        .expect("the folded-over flap")
        .id;
    let before_area = paper_area(&before_cp, &before_faces);

    // Both layers occupy x=0..0.5, but only the selected folded-over flap is
    // creased. The final placement is the same as before this operation.
    let result = crease_only(
        &mut document.cp,
        &before_faces,
        &before_state,
        &CreaseOnlyInput {
            line: [[0.0, 0.5], [0.5, 0.5]],
            movable_side_point: [0.25, 0.75],
            target_layers: Some(vec![selected]),
            direction: FoldDirection::Up,
        },
    )
    .expect("crease and immediately unfold the selected flap");

    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(
        !result.added_edges.is_empty(),
        "a permanent crease is added"
    );
    assert!(
        result
            .step
            .drivers
            .iter()
            .all(|driver| driver.target_angle_deg.abs() < 1e-12),
        "crease-only replay must end at zero degrees: {:?}",
        result.step.drivers
    );
    assert!(
        result.added_edges.iter().all(|id| {
            document
                .cp
                .edges
                .iter()
                .find(|edge| edge.id == *id)
                .is_some_and(|edge| edge.kind == EdgeKind::Mountain)
        }),
        "an Up crease on the mirrored flap is stored with the mirrored sense"
    );

    let after_faces = extract_faces(&document.cp);
    assert_eq!(
        after_faces.len(),
        3,
        "only the selected one of the two coincident flaps is split"
    );
    assert!((paper_area(&document.cp, &after_faces) - before_area).abs() < 1e-12);

    let mut children_per_parent: HashMap<_, usize> = HashMap::new();
    for face in &after_faces {
        let point = representative_point(&document.cp, face);
        let parent = before_faces
            .iter()
            .find(|candidate| point_in_face(&before_cp, candidate, point))
            .expect("every resulting face retains an original parent");
        *children_per_parent.entry(parent.id).or_default() += 1;
        assert_eq!(
            result.source_face_of.get(&face.id),
            Some(&parent.id),
            "crease-only provenance must retain the material parent for face {}",
            face.id
        );
        assert!(
            result.state.placements[&face.id].approx_eq(&before_state.placements[&parent.id], 1e-9),
            "face {} moved during a crease-only operation",
            face.id
        );
    }
    assert_eq!(children_per_parent.get(&selected), Some(&2));
    assert_eq!(children_per_parent.values().sum::<usize>(), 3);
    assert_eq!(
        children_per_parent
            .values()
            .filter(|&&count| count == 1)
            .count(),
        1,
        "the unselected flap remains one face"
    );

    append_step(&mut document, result.step.clone());
    let (replayed_state, state_warnings) =
        flat_state_at(&document, &after_faces, document.sequence.len())
            .expect("crease-only step has a replayable flat state");
    assert!(state_warnings.is_empty(), "{:?}", state_warnings);
    assert_eq!(replayed_state.order, result.state.order);
    for face in &after_faces {
        assert!(
            replayed_state.placements[&face.id].approx_eq(&result.state.placements[&face.id], 1e-9),
            "face {} differs after replay",
            face.id
        );
    }

    let replayed = replay(&document, document.sequence.len(), 1.0);
    assert!(replayed.skipped.is_empty(), "{:?}", replayed.skipped);
    assert!(replayed.warnings.is_empty(), "{:?}", replayed.warnings);
    assert_eq!(replayed.frame.faces.len(), after_faces.len());
    let gap = max_seam_gap(&document.cp, &after_faces, &replayed.frame);
    assert!(
        gap < 1e-6,
        "crease-only operation tore the paper: {gap:.3e}"
    );
}

#[test]
fn reverse_open_crease_sense_keeps_identity_provenance() {
    let mut document = Document::new(Paper { width_mm: 100.0, height_mm: 100.0 });
    let initial_faces = extract_faces(&document.cp);
    let initial_state = FlatState::initial(&document.cp, &initial_faces);
    let created = crease_only(
        &mut document.cp,
        &initial_faces,
        &initial_state,
        &CreaseOnlyInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            movable_side_point: [0.75, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("create an open crease");
    let faces = extract_faces(&document.cp);
    let result = reverse_open_crease_sense(
        &mut document.cp,
        &faces,
        &created.state,
        &ReverseOpenCreaseInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            target_layers: None,
        },
    )
    .expect("reverse the open crease");
    assert_eq!(result.source_face_of.len(), faces.len());
    for face in faces {
        assert_eq!(result.source_face_of.get(&face.id), Some(&face.id));
    }
}
