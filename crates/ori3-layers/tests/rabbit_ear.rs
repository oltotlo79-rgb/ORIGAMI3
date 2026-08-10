use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment, validate};
use ori3_geometry::{dist_point_segment, point_on_segment};
use ori3_layers::{FlatState, FoldDirection, RabbitEarInput, flat_state_at, rabbit_ear, replay};
use ori3_model::{CreasePattern, Document, EdgeKind, FoldStep, Paper, TechniqueKind};
use ori3_rigid::max_seam_gap;

fn symmetric_lines(mirrored: bool) -> [[[f64; 2]; 2]; 3] {
    let tangent = std::f64::consts::SQRT_2 - 1.0;
    let lines = [
        [[0.0, 0.0], [1.0, tangent]],
        [[0.0, 0.0], [1.0, 1.0]],
        [[0.0, 0.0], [tangent, 1.0]],
    ];
    if mirrored {
        lines.map(|line| line.map(|[x, y]| [1.0 - x, y]))
    } else {
        lines
    }
}

fn append_step(document: &mut Document, mut step: FoldStep) {
    step.id = u32::try_from(document.sequence.len()).expect("step ID fits in u32");
    document.sequence.push(step);
}

fn face_area(cp: &CreasePattern, face: &Face) -> f64 {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let points = face
        .vertices
        .iter()
        .map(|vertex| positions[vertex])
        .collect::<Vec<_>>();
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
        .abs()
        * 0.5
}

fn crease_kind(cp: &CreasePattern, line: [[f64; 2]; 2]) -> EdgeKind {
    let a = DVec2::from(line[0]);
    let b = DVec2::from(line[1]);
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let kinds = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .filter(|edge| {
            let p = positions[&edge.v0];
            let q = positions[&edge.v1];
            dist_point_segment((p + q) * 0.5, a, b) < 1e-7
                || (point_on_segment(p, a, b) && point_on_segment(q, a, b))
        })
        .map(|edge| edge.kind)
        .collect::<Vec<_>>();
    let first = *kinds.first().expect("the crease ray remains in the CP");
    assert!(
        kinds.iter().all(|kind| *kind == first),
        "one crease sense per rabbit-ear ray"
    );
    first
}

fn run_symmetric_case(mirrored: bool) -> (Vec<f64>, Vec<EdgeKind>, f64) {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let lines = symmetric_lines(mirrored);
    for line in lines {
        insert_segment(&mut document.cp, line[0], line[1], EdgeKind::Aux);
    }

    let before_faces = extract_faces(&document.cp);
    assert_eq!(before_faces.len(), 1, "auxiliary guides do not split faces");
    let state = FlatState::initial(&document.cp, &before_faces);
    let result = rabbit_ear(
        &mut document.cp,
        &before_faces,
        &state,
        &RabbitEarInput {
            creases: lines,
            target_layers: vec![before_faces[0].id],
            direction: FoldDirection::Up,
        },
    )
    .expect("the symmetric rabbit ear folds");

    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.step.kind, TechniqueKind::Pleat);
    assert_eq!(result.step.drivers.len(), 3, "one driver per crease");
    assert!(
        result
            .step
            .drivers
            .iter()
            .all(|driver| (driver.target_angle_deg.abs() - 180.0).abs() < 1e-9)
    );

    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 4, "three rays preserve all four paper sectors");
    assert!(validate(&document.cp).is_empty(), "valid crease pattern");
    let total_area = faces
        .iter()
        .map(|face| face_area(&document.cp, face))
        .sum::<f64>();
    assert!((total_area - 1.0).abs() < 1e-9, "paper area is preserved");

    append_step(&mut document, result.step.clone());
    let (flat, flat_warnings) = flat_state_at(&document, &faces, document.sequence.len())
        .expect("rabbit-ear result remains flat");
    assert!(flat_warnings.is_empty(), "{:?}", flat_warnings);
    assert_eq!(flat.order, result.state.order);
    for face in &faces {
        assert!(
            flat.placements[&face.id].approx_eq(&result.state.placements[&face.id], 1e-7),
            "face {} replays to the recorded flat placement",
            face.id
        );
    }

    let replayed = replay(&document, document.sequence.len(), 1.0);
    assert!(replayed.skipped.is_empty(), "{:?}", replayed.skipped);
    assert!(replayed.warnings.is_empty(), "{:?}", replayed.warnings);
    assert!(
        replayed.frame.warnings.is_empty(),
        "{:?}",
        replayed.frame.warnings
    );
    assert_eq!(replayed.frame.faces.len(), faces.len());
    assert!(
        replayed
            .frame
            .faces
            .iter()
            .all(|face| face
                .polygon
                .iter()
                .all(|point| DVec3::from(*point).z.abs() < 1e-6)),
        "the completed rabbit ear is flat"
    );
    let gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
    assert!(gap < 1e-6, "rabbit ear tore a seam: {gap:.3e}");

    let mut areas = faces
        .iter()
        .map(|face| face_area(&document.cp, face))
        .collect::<Vec<_>>();
    areas.sort_by(f64::total_cmp);
    let kinds = lines.map(|line| crease_kind(&document.cp, line)).to_vec();
    (areas, kinds, gap)
}

#[test]
fn symmetric_clockwise_and_counterclockwise_rabbit_ears_stay_flat_and_joined() {
    let left = run_symmetric_case(false);
    let right = run_symmetric_case(true);
    for (a, b) in left.0.iter().zip(&right.0) {
        assert!((a - b).abs() < 1e-9, "mirrored sector areas agree");
    }
    assert_eq!(
        left.1, right.1,
        "mirrored folds have the same crease senses"
    );
    assert!(left.2 < 1e-6 && right.2 < 1e-6);
}

#[test]
fn rabbit_ear_rejects_nonconcurrent_lines_and_missing_layers_atomically() {
    let document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&document.cp);
    let state = FlatState::initial(&document.cp, &faces);
    let original = document.cp.clone();

    let mut nonconcurrent = symmetric_lines(false);
    nonconcurrent[2] = [[0.0, 0.1], [std::f64::consts::SQRT_2 - 1.0, 1.0]];
    let mut cp = original.clone();
    let error = rabbit_ear(
        &mut cp,
        &faces,
        &state,
        &RabbitEarInput {
            creases: nonconcurrent,
            target_layers: vec![faces[0].id],
            direction: FoldDirection::Up,
        },
    )
    .expect_err("nonconcurrent crease rays are not a rabbit ear");
    assert!(error.contains("common vertex"), "{error}");
    assert_eq!(cp, original, "failed geometry leaves the CP untouched");

    let mut cp = original.clone();
    let error = rabbit_ear(
        &mut cp,
        &faces,
        &state,
        &RabbitEarInput {
            creases: symmetric_lines(false),
            target_layers: Vec::new(),
            direction: FoldDirection::Up,
        },
    )
    .expect_err("an empty local layer selection is insufficient");
    assert!(error.contains("target layer"), "{error}");
    assert_eq!(
        cp, original,
        "failed layer selection leaves the CP untouched"
    );

    let mut cp = original.clone();
    let error = rabbit_ear(
        &mut cp,
        &faces,
        &state,
        &RabbitEarInput {
            creases: symmetric_lines(false),
            target_layers: vec![u32::MAX],
            direction: FoldDirection::Up,
        },
    )
    .expect_err("a missing local layer is insufficient");
    assert!(error.contains("does not exist"), "{error}");
    assert_eq!(cp, original, "failed layer lookup leaves the CP untouched");
}
