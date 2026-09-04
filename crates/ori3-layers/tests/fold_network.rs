use ori3_cp::{extract_faces, insert_segment};
use ori3_layers::{ReverseFoldNetworkInput, flat_state_at, replay, reverse_fold_network};
use ori3_model::{Document, DriverLine, EdgeKind, FoldStep, Paper, TechniqueKind};
use ori3_rigid::max_seam_gap;

fn accordion() -> Document {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let mut drivers = Vec::new();
    for (index, x) in [0.2, 0.4, 0.6, 0.8].into_iter().enumerate() {
        let kind = if index % 2 == 0 {
            EdgeKind::Valley
        } else {
            EdgeKind::Mountain
        };
        insert_segment(&mut document.cp, [x, 0.0], [x, 1.0], kind);
        drivers.push(DriverLine {
            a: [x, 0.0],
            b: [x, 1.0],
            target_angle_deg: if kind == EdgeKind::Mountain {
                180.0
            } else {
                -180.0
            },
        });
    }
    let faces = extract_faces(&document.cp);
    let points = faces
        .iter()
        .map(|face| ori3_layers::representative_point(&document.cp, face))
        .collect();
    document.sequence.push(FoldStep {
        id: 0,
        kind: TechniqueKind::Pleat,
        drivers,
        layer_order: Some(points),
        alignment: None,
        finish_soft: None,
        note: String::new(),
        technique_classification: None,
    });
    document
}

#[test]
fn four_crease_local_accordion_reverses_atomically_without_moving_its_outline() {
    let mut document = accordion();
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 5);
    let (before, warnings) = flat_state_at(&document, &faces, 1).unwrap();
    assert!(warnings.is_empty());
    let before_kinds = document
        .cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| (edge.id, edge.kind))
        .collect::<std::collections::HashMap<_, _>>();
    // The four material hinges alternate between the two visible sides of the W stack.
    let left = [[0.8, 0.0], [0.8, 1.0]];
    let right = [[1.0, 0.0], [1.0, 1.0]];
    let result = reverse_fold_network(
        &mut document.cp,
        &faces,
        &before,
        &ReverseFoldNetworkInput {
            target_layers: before.order.clone(),
            creases: vec![left, right, left, right],
        },
    )
    .unwrap();
    assert!(result.warnings.is_empty());
    assert_eq!(
        result.state.order,
        before.order.iter().rev().copied().collect::<Vec<_>>()
    );
    for face in &faces {
        assert!(result.state.placements[&face.id].approx_eq(&before.placements[&face.id], 1e-9));
    }
    for edge in document
        .cp
        .edges
        .iter()
        .filter(|edge| before_kinds.contains_key(&edge.id))
    {
        let expected = match before_kinds[&edge.id] {
            EdgeKind::Mountain => EdgeKind::Valley,
            EdgeKind::Valley => EdgeKind::Mountain,
            other => other,
        };
        assert_eq!(edge.kind, expected);
    }

    let mut step = result.step.clone();
    step.id = 1;
    document.sequence.push(step);
    let (after, warnings) = flat_state_at(&document, &faces, 2).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(after.order, result.state.order);
    for face in &faces {
        assert!(after.placements[&face.id].approx_eq(&result.state.placements[&face.id], 1e-7));
    }
    let replayed = replay(&document, 2, 1.0);
    assert!(replayed.skipped.is_empty());
    assert!(replayed.warnings.is_empty());
    assert!(replayed.frame.warnings.is_empty());
    assert_eq!(replayed.frame.faces.len(), faces.len());
    assert!(max_seam_gap(&document.cp, &faces, &replayed.frame) < 1e-6);
}
