//! G2 minimal reproductions of the first invalid operation-built bird/frog poses.
//!
//! Two simple folds make the smallest four-layer square in the source operation
//! prefixes. The two zero-angle squash restacks are unnecessary to reproduce
//! these failures. These tests intentionally record the current defects for G3.
use std::collections::{BTreeSet, HashMap};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::precrease_collapse::{
    PrecreaseOrderValidation, PrecreaseStackDiagnosis, PrecreaseStackSatisfiability,
    diagnose_precrease_layer_order_at_angles, validate_precrease_layer_order_at_angles,
};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{FlatState, FoldThroughResult, flat_state_at, petal, squash};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, Paper};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: None,
            direction,
        },
    )
    .expect("折れる指定");
    assert!(
        res.warnings.is_empty(),
        "警告なしで折れる: {:?}",
        res.warnings
    );
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

fn apply(
    doc: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
    open_to_back: Option<bool>,
) -> FlatState {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back,
            polygon: None,
            center: None,
        },
    )
    .expect("折れる指定");
    assert!(
        res.warnings.is_empty(),
        "警告なしで折れる: {:?}",
        res.warnings
    );
    let new_faces = extract_faces(&cp);
    assert_eq!(res.source_face_of.len(), new_faces.len());
    assert!(
        new_faces
            .iter()
            .all(|face| res.source_face_of.contains_key(&face.id))
    );
    assert!(
        res.source_face_of
            .values()
            .all(|parent| faces.iter().any(|face| face.id == *parent))
    );
    assert!(
        faces
            .iter()
            .all(|face| res.source_face_of.values().any(|parent| *parent == face.id))
    );
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
    res.state
}

fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, warnings) = flat_state_at(doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    (faces, state)
}

fn vertex_pos(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

fn spine_to(doc: &Document, mid: [f64; 2]) -> ([[f64; 2]; 2], [FaceId; 2]) {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    let (center, m) = (DVec2::new(0.5, 0.5), DVec2::from(mid));
    let mut edge_faces: HashMap<u32, Vec<FaceId>> = HashMap::new();
    for f in &faces {
        for e in &f.edges {
            edge_faces.entry(*e).or_default().push(f.id);
        }
    }
    let rank = |id: &FaceId| {
        state
            .order
            .iter()
            .position(|x| x == id)
            .expect("層順序の面")
    };
    for e in &doc.cp.edges {
        let (Some(&p0), Some(&p1)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
            continue;
        };
        let same = |a: DVec2, b: DVec2| (a - b).length() < 1e-9;
        if !((same(p0, center) && same(p1, m)) || (same(p1, center) && same(p0, m))) {
            continue;
        }
        let fs = edge_faces.get(&e.id).expect("背に面がある");
        assert_eq!(fs.len(), 2, "背は2層をつなぐ");
        let pl = state.placements[&fs[0]];
        let (a, b) = (pl.apply(p0), pl.apply(p1));
        let (lo, hi) = if rank(&fs[0]) < rank(&fs[1]) {
            (fs[0], fs[1])
        } else {
            (fs[1], fs[0])
        };
        return ([[a.x, a.y], [b.x, b.y]], [lo, hi]);
    }
    panic!("紙の中心から {mid:?} への折り目が見つからない");
}

#[derive(Debug)]
struct LocalFailures {
    maekawa: Vec<u32>,
    kawasaki: Vec<u32>,
    blb: Vec<(u32, u32, u32)>,
}

fn local_failures(cp: &CreasePattern, angles: &HashMap<u32, f64>) -> LocalFailures {
    let border = cp
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Border)
        .flat_map(|e| [e.v0, e.v1])
        .collect::<BTreeSet<_>>();
    let pos = vertex_pos(cp);
    let mut result = LocalFailures {
        maekawa: Vec::new(),
        kawasaki: Vec::new(),
        blb: Vec::new(),
    };
    for vertex in cp.vertices.iter().filter(|v| !border.contains(&v.id)) {
        let mut rays = cp
            .edges
            .iter()
            .filter(|e| {
                (e.v0 == vertex.id || e.v1 == vertex.id)
                    && angles.get(&e.id).is_some_and(|a| a.abs() > 90.)
            })
            .map(|e| {
                let other = if e.v0 == vertex.id { e.v1 } else { e.v0 };
                let d = pos[&other] - pos[&vertex.id];
                (d.y.atan2(d.x), e.id)
            })
            .collect::<Vec<_>>();
        if rays.is_empty() {
            continue;
        }
        rays.sort_by(|a, b| a.0.total_cmp(&b.0));
        let sectors = (0..rays.len())
            .map(|i| (rays[(i + 1) % rays.len()].0 - rays[i].0).rem_euclid(std::f64::consts::TAU))
            .collect::<Vec<_>>();
        let sum = rays
            .iter()
            .map(|(_, id)| if angles[id] > 0. { 1_i32 } else { -1 })
            .sum::<i32>();
        if sum.abs() != 2 {
            result.maekawa.push(vertex.id);
        }
        if rays.len() % 2 != 0
            || (sectors.iter().step_by(2).sum::<f64>() - std::f64::consts::PI).abs()
                > ori3_model::EPS
        {
            result.kawasaki.push(vertex.id);
        }
        for i in 0..rays.len() {
            if sectors[i] + ori3_model::EPS < sectors[(i + rays.len() - 1) % rays.len()]
                && sectors[i] + ori3_model::EPS < sectors[(i + 1) % rays.len()]
                && angles[&rays[i].1] * angles[&rays[(i + 1) % rays.len()].1] > 0.
            {
                result
                    .blb
                    .push((vertex.id, rays[i].1, rays[(i + 1) % rays.len()].1));
            }
        }
    }
    result
}

/// Check actual replay angles, independently of the CP's declared crease kinds.
fn measure(
    doc: &Document,
) -> (
    LocalFailures,
    PrecreaseStackDiagnosis,
    PrecreaseOrderValidation,
) {
    let (faces, state) = state_of(doc);
    let shown = ori3_layers::replay_with_faces(doc, &faces, doc.sequence.len(), 1.0);
    assert!(shown.warnings.is_empty(), "{:?}", shown.warnings);
    assert!(shown.skipped.is_empty());
    // Same seam limit as the source bird/frog acceptance tests; observed <1e-14.
    assert!(max_seam_gap(&doc.cp, &faces, &shown.frame) < 1e-6);
    assert!(self_intersection_pairs(&shown.frame).is_empty());
    assert!(shown.hinge_angles.values().all(|angle| {
        angle.abs() <= ori3_model::EPS || (angle.abs() - 180.0).abs() <= ori3_model::EPS
    }));
    let local = local_failures(&doc.cp, &shown.hinge_angles);
    let diagnosis = diagnose_precrease_layer_order_at_angles(
        &doc.cp,
        &faces,
        &state.placements,
        &shown.hinge_angles,
    )
    .unwrap();
    let saved = validate_precrease_layer_order_at_angles(
        &doc.cp,
        &faces,
        &state.placements,
        &shown.hinge_angles,
        &state.order,
    )
    .unwrap();
    (local, diagnosis, saved)
}

fn four_layer_square() -> Document {
    let mut doc = square_doc();
    for (line, keep) in [
        ([[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]),
        ([[0.5, 0.0], [0.5, 0.5]], [0.25, 0.25]),
    ] {
        fold(&mut doc, line, keep, FoldDirection::Up);
        let (local, diagnosis, saved) = measure(&doc);
        assert!(local.maekawa.is_empty());
        assert!(local.kawasaki.is_empty());
        assert!(local.blb.is_empty());
        assert!(matches!(
            diagnosis.satisfiability,
            PrecreaseStackSatisfiability::Sat { .. }
        ));
        assert!(saved.is_valid());
        assert!(saved.discarded_relations.is_empty());
    }
    assert_eq!(
        (
            doc.cp.vertices.len(),
            doc.cp.edges.len(),
            extract_faces(&doc.cp).len()
        ),
        (9, 12, 4)
    );
    doc
}

#[test]
fn bird_petal_first_breakdown_needs_only_two_simple_folds() {
    let mut doc = four_layer_square();
    let (_, state) = state_of(&doc);
    let front = vec![*state.order.last().unwrap()];
    apply(
        &mut doc,
        petal,
        front,
        [[0.0, 1.0], [0.5, 0.5]],
        [0.0, 1.0],
        None,
    );
    assert_eq!(doc.sequence.len(), 3);
    assert_eq!(
        (
            doc.cp.vertices.len(),
            doc.cp.edges.len(),
            extract_faces(&doc.cp).len()
        ),
        (11, 19, 9)
    );
    let (local, diagnosis, saved) = measure(&doc);
    assert!(local.kawasaki.is_empty());
    assert!(local.blb.is_empty());
    // G3: once pocket placement is repaired, change these defect expectations
    // to no Maekawa failures, SAT with no core, valid saved order and no discards.
    assert_eq!(local.maekawa, [9, 10]);
    match diagnosis.satisfiability {
        PrecreaseStackSatisfiability::Unsat { minimal_rules } => assert_eq!(minimal_rules.len(), 8),
        PrecreaseStackSatisfiability::Sat { .. } => {
            panic!("G3 must update the recorded bird defect")
        }
    }
    assert!(saved.violations.adjacent_folds.is_empty());
    assert_eq!(saved.violations.taco_tortilla.len(), 2);
    assert_eq!(saved.violations.continuous_crossings.len(), 2);
    assert!(saved.violations.taco_taco.is_empty());
    assert!(saved.violations.continuous.is_empty());
    assert_eq!(saved.discarded_relations.len(), 8);
}

#[test]
fn frog_outer_pocket_first_breakdown_needs_only_two_simple_folds() {
    let mut doc = four_layer_square();
    // Before the redundant restacks, the outer pocket is the bottom median
    // instead of the top median used by acceptance_frog's fifth operation.
    let (line, pocket) = spine_to(&doc, [0.5, 0.0]);
    let (_, state) = state_of(&doc);
    assert_eq!(pocket, [state.order[0], *state.order.last().unwrap()]);
    apply(
        &mut doc,
        squash,
        pocket.to_vec(),
        line,
        [0.0, 1.0],
        Some(true),
    );
    assert_eq!(doc.sequence.len(), 3);
    assert_eq!(
        (
            doc.cp.vertices.len(),
            doc.cp.edges.len(),
            extract_faces(&doc.cp).len()
        ),
        (11, 16, 6)
    );
    let (local, diagnosis, saved) = measure(&doc);
    assert!(local.maekawa.is_empty());
    assert!(local.kawasaki.is_empty());
    assert!(local.blb.is_empty());
    // G3: once the cyclic outer pocket remains a connected block, expect SAT
    // with no core, a valid saved order and no discarded relations here.
    match diagnosis.satisfiability {
        PrecreaseStackSatisfiability::Unsat { minimal_rules } => assert_eq!(minimal_rules.len(), 6),
        PrecreaseStackSatisfiability::Sat { .. } => {
            panic!("G3 must update the recorded frog defect")
        }
    }
    assert!(saved.violations.adjacent_folds.is_empty());
    assert_eq!(
        saved.violations.taco_tortilla,
        [(4, 5, 0), (4, 5, 1), (4, 5, 2), (4, 5, 3)]
    );
    assert!(saved.violations.continuous_crossings.is_empty());
    assert!(saved.violations.taco_taco.is_empty());
    assert!(saved.violations.continuous.is_empty());
    assert_eq!(saved.discarded_relations.len(), 8);
}
