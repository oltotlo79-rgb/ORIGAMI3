//! 完全平坦な折り上がりへ重なり防止を適用しても、物理フレームを歪めず
//! 貫通警告の根拠となる自己交差を作らないことの回帰テスト。

use ori3_cp::{Face, extract_faces, local_violations};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{flat_state_at, replay, squash};
use ori3_model::{CreasePattern, Document, Driver, Edge, EdgeKind, FaceId, Paper, Vertex};
use ori3_rigid::{max_seam_gap, self_intersection_pairs, solve};
use ori3_soft::{OverlapSettings, prevent_overlap};

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

/// 正方形の中心から45度刻みで8本の折り目が伸びる米字CP。
///
/// 対角線4半直線を谷、水平・垂直4半直線を山にした4M/4Vから、下向きの
/// 垂直半直線だけを谷へ反転した3M/5V。前川・川崎の局所検査を満たす。
fn flat_foldable_kome() -> CreasePattern {
    let radial_kinds = [
        EdgeKind::Valley,
        EdgeKind::Valley,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Valley,
        EdgeKind::Mountain,
    ];
    let mut edges = vec![
        edge(0, 0, 1, EdgeKind::Border),
        edge(1, 1, 2, EdgeKind::Border),
        edge(2, 2, 3, EdgeKind::Border),
        edge(3, 3, 4, EdgeKind::Border),
        edge(4, 4, 5, EdgeKind::Border),
        edge(5, 5, 6, EdgeKind::Border),
        edge(6, 6, 7, EdgeKind::Border),
        edge(7, 7, 0, EdgeKind::Border),
    ];
    edges.extend(
        radial_kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| edge(8 + index as u32, 8, index as u32, kind)),
    );
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 0.5, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 0.5),
            vertex(4, 1.0, 1.0),
            vertex(5, 0.5, 1.0),
            vertex(6, 0.0, 1.0),
            vertex(7, 0.0, 0.5),
            vertex(8, 0.5, 0.5),
        ],
        edges,
        next_vertex_id: 9,
        next_edge_id: 16,
    }
}

/// 正方形を縦3短冊へ分けた木構造。104度では剛体解に交差がない一方、従来の
/// FaceId順PBDは外側2面の交差を新しく作っていた診断用の最小形。
fn three_strips() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0 / 3.0, 0.0),
            vertex(2, 2.0 / 3.0, 0.0),
            vertex(3, 1.0, 0.0),
            vertex(4, 1.0, 1.0),
            vertex(5, 2.0 / 3.0, 1.0),
            vertex(6, 1.0 / 3.0, 1.0),
            vertex(7, 0.0, 1.0),
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
            edge(8, 1, 6, EdgeKind::Mountain),
            edge(9, 2, 5, EdgeKind::Mountain),
        ],
        next_vertex_id: 8,
        next_edge_id: 10,
    }
}

fn z_span(frame: &ori3_model::Frame3D) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for point in frame.faces.iter().flat_map(|face| &face.polygon) {
        min = min.min(point[2]);
        max = max.max(point[2]);
    }
    if min.is_finite() && max.is_finite() {
        max - min
    } else {
        0.0
    }
}

fn polygons(frame: &ori3_model::Frame3D) -> Vec<Vec<[f64; 3]>> {
    frame
        .faces
        .iter()
        .map(|face| face.polygon.clone())
        .collect()
}

#[test]
fn flat_foldable_kome_stays_flat_and_has_no_intersections_after_overlap_prevention() {
    let cp = flat_foldable_kome();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 8, "米字は8三角形");
    assert!(
        local_violations(&cp).is_empty(),
        "3M/5Vは前川・川崎の局所検査を満たす"
    );

    let drivers: Vec<Driver> = cp
        .edges
        .iter()
        .filter_map(|crease| {
            let target_angle_deg = match crease.kind {
                EdgeKind::Mountain => 180.0,
                EdgeKind::Valley => -180.0,
                _ => return None,
            };
            Some(Driver {
                hinge: crease.id,
                target_angle_deg,
            })
        })
        .collect();
    assert_eq!(drivers.len(), 8);

    let solved = solve(&cp, &faces, &drivers, None);
    assert!(solved.converged, "全折り目±180度で収束する");
    assert!(
        max_seam_gap(&cp, &faces, &solved.frame) < 1e-9,
        "生の剛体解の裂けは1e-9未満"
    );
    assert!(
        z_span(&solved.frame) < 1e-12,
        "生の剛体解は完全平坦: z幅={}",
        z_span(&solved.frame)
    );
    assert!(
        self_intersection_pairs(&solved.frame).is_empty(),
        "生の剛体解には自己交差がない"
    );

    let mut corrected = solved.frame;
    let before = polygons(&corrected);
    let face_id_order: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    let report = prevent_overlap(
        &cp,
        &faces,
        &mut corrected,
        &face_id_order,
        &face_id_order,
        0.5,
        &OverlapSettings::default(),
    );

    let warning_pairs = self_intersection_pairs(&corrected);
    assert!(
        z_span(&corrected) < 1e-6,
        "重なり防止後も平坦基準内: z幅={}",
        z_span(&corrected)
    );
    assert!(
        warning_pairs.is_empty(),
        "重なり防止が貫通警告の根拠を人工的に作らない: {warning_pairs:?}"
    );
    assert!(
        !report.applied,
        "完全平坦なら重なり防止を適用しない: {report:?}"
    );
    assert_eq!(
        polygons(&corrected),
        before,
        "完全平坦な物理フレームを歪めない"
    );
}

#[test]
fn nonflat_raw_zero_stays_zero_when_the_pbd_candidate_adds_an_intersection() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let solved = solve(
        &cp,
        &faces,
        &[
            Driver {
                hinge: 8,
                target_angle_deg: 150.0,
            },
            Driver {
                hinge: 9,
                target_angle_deg: 104.0,
            },
        ],
        None,
    );
    assert!(solved.converged);
    assert!(
        self_intersection_pairs(&solved.frame).is_empty(),
        "診断どおり104度の剛体解は交差0"
    );

    let mut corrected = solved.frame;
    let before = polygons(&corrected);
    let order: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    let report = prevent_overlap(
        &cp,
        &faces,
        &mut corrected,
        &order,
        &order,
        0.5,
        &OverlapSettings::default(),
    );

    assert!(report.skipped_untrusted_layer_order, "{report:?}");
    assert!(!report.attempted, "物理層順なしではPBDを始めない");
    assert!(!report.accepted);
    assert!(!report.applied);
    assert_eq!(report.intersection_pairs_before, 0);
    assert_eq!(report.intersection_pairs_after, 0);
    assert_eq!(polygons(&corrected), before, "未信頼順は入力をbitwise保持");
    assert!(self_intersection_pairs(&corrected).is_empty());
}

#[test]
fn overlap_output_never_replaces_an_existing_intersection_with_a_new_pair() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let mut solved = solve(
        &cp,
        &faces,
        &[
            Driver {
                hinge: 8,
                target_angle_deg: 150.0,
            },
            Driver {
                hinge: 9,
                target_angle_deg: 110.0,
            },
        ],
        None,
    );
    assert!(solved.converged);
    for face in &mut solved.frame.faces {
        face.layer = face.face;
    }
    let before_pairs = self_intersection_pairs(&solved.frame);
    assert_eq!(before_pairs.len(), 1, "110度では外側2面が交差する");

    let mut corrected = solved.frame;
    let before_polygons = polygons(&corrected);
    let order: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    let report = prevent_overlap(
        &cp,
        &faces,
        &mut corrected,
        &order,
        &order,
        0.5,
        &OverlapSettings::default(),
    );
    let after_pairs = self_intersection_pairs(&corrected);

    assert!(
        report.attempted || report.skipped_no_signed_penetration,
        "{report:?}"
    );
    assert!(after_pairs.len() <= before_pairs.len(), "{report:?}");
    assert!(
        after_pairs.iter().all(|pair| before_pairs.contains(pair)),
        "既存ペアを別の新規ペアへ置換しない: before={before_pairs:?}, after={after_pairs:?}"
    );
    assert!(
        report.total_depth_after <= report.total_depth_before + 1e-12,
        "{report:?}"
    );
    assert!(
        report.max_depth_after <= report.max_depth_before + 1e-12,
        "{report:?}"
    );
    if !report.accepted {
        assert_eq!(polygons(&corrected), before_polygons);
    }
}

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, warnings) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    assert!(warnings.is_empty(), "折る前の警告なし: {warnings:?}");
    let mut cp = doc.cp.clone();
    let result = fold_through(
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
    assert!(result.warnings.is_empty(), "折り操作の警告なし");
    let mut step = result.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

fn squash_bottom(doc: &mut Document, line: [[f64; 2]; 2], reference_point: [f64; 2]) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, warnings) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    assert!(warnings.is_empty(), "つぶし折り前の警告なし: {warnings:?}");
    let mut cp = doc.cp.clone();
    let result = squash(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap: vec![state.order[0]],
            line,
            reference_point,
            open_to_back: None,
            polygon: None,
            center: None,
        },
    )
    .expect("開いてつぶせる指定");
    assert!(result.warnings.is_empty(), "つぶし折りの警告なし");
    let mut step = result.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

/// `ori3-layers/tests/acceptance_crane.rs::preliminary_base` と同じ折り順。
fn preliminary_base() -> Document {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.0, 0.5], [1.0, 0.5]],
        [0.5, 0.25],
        FoldDirection::Up,
    );
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 0.5]],
        [0.25, 0.25],
        FoldDirection::Up,
    );
    squash_bottom(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]);
    squash_bottom(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]);
    doc
}

#[test]
fn crane_preliminary_base_has_zero_penetration_warnings_when_fully_folded() {
    let doc = preliminary_base();
    let faces: Vec<Face> = extract_faces(&doc.cp);
    let replayed = replay(&doc, doc.sequence.len(), 1.0);
    assert!(
        replayed.warnings.is_empty(),
        "再生警告なし: {:?}",
        replayed.warnings
    );
    assert!(
        replayed.skipped.is_empty(),
        "飛ばした手順なし: {:?}",
        replayed.skipped
    );
    assert_eq!(faces.len(), 4, "鶴の予備基本形は4層");
    assert!(
        doc.cp
            .edges
            .iter()
            .filter(|crease| matches!(crease.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .all(|crease| {
                replayed
                    .hinge_angles
                    .get(&crease.id)
                    .is_some_and(|angle| (angle.abs() - 180.0).abs() < 1e-6)
            }),
        "鶴の予備基本形は全折り目を±180度まで畳む: {:?}",
        replayed.hinge_angles
    );
    assert!(z_span(&replayed.frame) < 1e-12, "折り上がりは完全平坦");
    assert!(
        self_intersection_pairs(&replayed.frame).is_empty(),
        "生の鶴の予備基本形には貫通警告の根拠がない"
    );

    // 手順を持たない角度操作と同じ、面ID順・進行度0.5の入力でも平坦形を歪めない。
    let face_id_order: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    let mut corrected = replayed.frame;
    let before = polygons(&corrected);
    let report = prevent_overlap(
        &doc.cp,
        &faces,
        &mut corrected,
        &face_id_order,
        &face_id_order,
        0.5,
        &OverlapSettings::default(),
    );
    let warning_pairs = self_intersection_pairs(&corrected);

    assert_eq!(
        warning_pairs.len(),
        0,
        "鶴の土台の貫通警告は0件: {warning_pairs:?}"
    );
    assert!(
        !report.applied,
        "完全平坦な鶴の土台へPBDを適用しない: {report:?}"
    );
    assert_eq!(
        polygons(&corrected),
        before,
        "鶴の土台の物理フレームを歪めない"
    );
}
