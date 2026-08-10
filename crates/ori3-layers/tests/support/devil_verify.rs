//! Per-book-step acceptance checks shared by the Devil sequence.

use std::collections::BTreeSet;

use ori3_cp::{extract_faces, validate};
use ori3_layers::{flat_state_at, replay};
use ori3_model::{Document, TechniqueKind};
use ori3_rigid::max_seam_gap;

/// Replay the just-completed book step and reject warnings, skips, lost faces, and seams.
pub fn verify_book_step(document: &Document, book_step: u32) -> f64 {
    assert!(
        validate(&document.cp).is_empty(),
        "手順{book_step}: CP検証警告={:?}",
        validate(&document.cp)
    );
    let faces = extract_faces(&document.cp);
    assert!(!faces.is_empty(), "手順{book_step}: 面がない");
    let replayed = replay(document, document.sequence.len(), 1.0);
    assert!(
        replayed.warnings.is_empty(),
        "手順{book_step}: replay warnings={:?}",
        replayed.warnings
    );
    assert!(
        replayed.skipped.is_empty(),
        "手順{book_step}: skipped={:?}",
        replayed.skipped
    );
    assert!(
        replayed.frame.warnings.is_empty(),
        "手順{book_step}: frame warnings={:?}",
        replayed.frame.warnings
    );
    assert_eq!(
        replayed.frame.faces.len(),
        faces.len(),
        "手順{book_step}: 3D frameから面が失われた"
    );
    let frame_ids = replayed
        .frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<BTreeSet<_>>();
    let topology_ids = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    assert_eq!(frame_ids, topology_ids, "手順{book_step}: 面IDが失われた");
    for face in &replayed.frame.faces {
        assert!(face.polygon.len() >= 3, "手順{book_step}: 面{}が退化", face.face);
        assert!(
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite()),
            "手順{book_step}: 面{}に非有限座標",
            face.face
        );
    }
    let gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
    assert!(gap < 1e-6, "手順{book_step}: max_seam_gap={gap:.9e}");

    let is_pose = document
        .sequence
        .last()
        .is_some_and(|step| step.kind == TechniqueKind::Pose);
    if !is_pose {
        let (state, warnings) = flat_state_at(document, &faces, document.sequence.len())
            .unwrap_or_else(|error| panic!("手順{book_step}: 平坦状態を復元できない: {error}"));
        assert!(warnings.is_empty(), "手順{book_step}: flat warnings={warnings:?}");
        assert_eq!(state.placements.len(), faces.len());
        assert_eq!(state.order.len(), faces.len());
        assert_eq!(
            state.order.iter().copied().collect::<BTreeSet<_>>(),
            topology_ids,
            "手順{book_step}: 層順序から面が失われた"
        );
    }

    if book_step.is_multiple_of(10) {
        println!(
            "手順{book_step}まで通過。max_seam_gap={gap:.3e}。faces={}",
            faces.len()
        );
    }
    gap
}
