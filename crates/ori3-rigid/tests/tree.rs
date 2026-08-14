//! Task 1-7: 全域木の角度伝播のテスト。
//!
//! ヒンジ角の規約(ori3-rigid::tree のdocコメント参照):
//! 紙の表を+zとし、+θ=山折り(動く面が表から見て奥(−z)側へ畳まれる)、
//! −θ=谷折り(手前(+z)側へ畳まれる)。

use std::collections::HashMap;

use glam::{DMat3, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_model::{CreasePattern, Driver, EPS, Edge, EdgeKind, Vertex};
use ori3_rigid::motion::solve_motion;
use ori3_rigid::{FoldedFrame, propagate, to_frame3d};

fn v(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn e(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

/// 正方形+中央縦1本(x=0.5)。辺ID6が山折りヒンジ。
fn split_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 0.5, 0.0),
            v(2, 1.0, 0.0),
            v(3, 1.0, 1.0),
            v(4, 0.5, 1.0),
            v(5, 0.0, 1.0),
        ],
        edges: vec![
            e(0, 0, 1, EdgeKind::Border),
            e(1, 1, 2, EdgeKind::Border),
            e(2, 2, 3, EdgeKind::Border),
            e(3, 3, 4, EdgeKind::Border),
            e(4, 4, 5, EdgeKind::Border),
            e(5, 5, 0, EdgeKind::Border),
            e(6, 1, 4, EdgeKind::Mountain),
        ],
        next_vertex_id: 6,
        next_edge_id: 7,
    }
}

fn is_identity(r: DMat3, t: DVec3) -> bool {
    (r - DMat3::IDENTITY)
        .to_cols_array()
        .iter()
        .all(|c| c.abs() < 1e-12)
        && t.length() < 1e-12
}

/// (固定面, 動く面) を faces の添字で返す。
fn fixed_and_moving(faces: &[Face], frame: &FoldedFrame) -> (usize, usize) {
    let fixed = faces
        .iter()
        .position(|f| {
            let (r, t) = frame.transforms[&f.id];
            is_identity(r, t)
        })
        .expect("恒等変換の固定面(根面)が存在するはず");
    let moving = faces.len() - 1 - fixed; // 2面前提
    (fixed, moving)
}

/// 動く面の変換後ポリゴンを返す(頂点順は face.vertices と同順)。
fn moved_polygon(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &FoldedFrame,
    idx: usize,
) -> Vec<DVec3> {
    let f3d = to_frame3d(cp, faces, frame);
    let face3d = f3d
        .faces
        .iter()
        .find(|f| f.face == faces[idx].id)
        .expect("面が出力に含まれるはず");
    face3d.polygon.iter().map(|p| DVec3::from(*p)).collect()
}

/// Newellの方法で多角形の法線(正規化済み)を求める。
fn normal_of(poly: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for i in 0..poly.len() {
        let p = poly[i];
        let q = poly[(i + 1) % poly.len()];
        n += p.cross(q);
    }
    n.normalize()
}

fn centroid(poly: &[DVec3]) -> DVec3 {
    poly.iter().copied().sum::<DVec3>() / poly.len() as f64
}

#[test]
fn fold_180_overlaps_mirrored() {
    let cp = split_square();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 2);
    let frame = propagate(&cp, &faces, &HashMap::from([(6u32, 180.0f64)]));
    let (_, moving) = fixed_and_moving(&faces, &frame);
    assert!(!frame.mirrored[&faces[1 - moving].id]);
    assert!(frame.mirrored[&faces[moving].id]);
    let poly = moved_polygon(&cp, &faces, &frame, moving);
    // 動く面の各頂点は、折り線 x=0.5 を挟んで固定面側へ鏡映された位置に来る
    for (vid, p3) in faces[moving].vertices.iter().zip(&poly) {
        let p2 = cp.vertices.iter().find(|vx| vx.id == *vid).unwrap().pos;
        let expected = DVec3::new(1.0 - p2[0], p2[1], 0.0);
        assert!(
            (*p3 - expected).length() < EPS,
            "頂点{vid}: {p3:?} != {expected:?}"
        );
        assert!(p3.z.abs() < EPS, "z差はEPS以内: z={}", p3.z);
    }
}

#[test]
fn fold_90_dihedral_is_90() {
    let cp = split_square();
    let faces = extract_faces(&cp);
    let frame = propagate(&cp, &faces, &HashMap::from([(6u32, 90.0f64)]));
    let (fixed, moving) = fixed_and_moving(&faces, &frame);
    let n_fixed = normal_of(&moved_polygon(&cp, &faces, &frame, fixed));
    let n_moving = normal_of(&moved_polygon(&cp, &faces, &frame, moving));
    // 二面角90° ⇔ 法線の内積が0
    assert!(
        n_fixed.dot(n_moving).abs() < 1e-12,
        "法線の内積: {}",
        n_fixed.dot(n_moving)
    );
}

#[test]
fn mountain_and_valley_fold_opposite_sides() {
    let cp = split_square();
    let faces = extract_faces(&cp);

    // 山折り(+90°): 動く面は表(+z)から見て奥(−z)側へ畳まれる
    let frame = propagate(&cp, &faces, &HashMap::from([(6u32, 90.0f64)]));
    let (_, moving) = fixed_and_moving(&faces, &frame);
    let c = centroid(&moved_polygon(&cp, &faces, &frame, moving));
    assert!(c.z < -0.1, "山折りは−z側のはず: z={}", c.z);

    // 谷折り(−90°): 動く面は手前(+z)側へ畳まれる
    let frame = propagate(&cp, &faces, &HashMap::from([(6u32, -90.0f64)]));
    let (_, moving) = fixed_and_moving(&faces, &frame);
    let c = centroid(&moved_polygon(&cp, &faces, &frame, moving));
    assert!(c.z > 0.1, "谷折りは+z側のはず: z={}", c.z);
}

#[test]
fn unspecified_hinge_defaults_to_flat() {
    let cp = split_square();
    let faces = extract_faces(&cp);
    // 角度を一切指定しない → 全面がxy平面のまま(恒等変換)
    let frame = propagate(&cp, &faces, &HashMap::new());
    for f in &faces {
        let (r, t) = frame.transforms[&f.id];
        assert!(is_identity(r, t), "面{}が動いてしまった", f.id);
    }
    assert!(frame.warnings.is_empty(), "warnings={:?}", frame.warnings);
}

#[test]
fn disconnected_faces_warn_but_do_not_stop() {
    // 離れた正方形2つ(折り線で繋がっていない)
    let cp = CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 1.0, 0.0),
            v(2, 1.0, 1.0),
            v(3, 0.0, 1.0),
            v(4, 2.0, 0.0),
            v(5, 3.0, 0.0),
            v(6, 3.0, 1.0),
            v(7, 2.0, 1.0),
        ],
        edges: vec![
            e(0, 0, 1, EdgeKind::Border),
            e(1, 1, 2, EdgeKind::Border),
            e(2, 2, 3, EdgeKind::Border),
            e(3, 3, 0, EdgeKind::Border),
            e(4, 4, 5, EdgeKind::Border),
            e(5, 5, 6, EdgeKind::Border),
            e(6, 6, 7, EdgeKind::Border),
            e(7, 7, 4, EdgeKind::Border),
        ],
        next_vertex_id: 8,
        next_edge_id: 8,
    };
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 2);
    let frame = propagate(&cp, &faces, &HashMap::new());
    assert!(!frame.warnings.is_empty(), "非連結の警告が出るはず");
    for f in &faces {
        let (r, t) = frame.transforms[&f.id];
        assert!(is_identity(r, t), "浮いた面は恒等変換のまま出す");
    }
    // Frame3Dにも警告が引き継がれ、全面が出力される
    let f3d = to_frame3d(&cp, &faces, &frame);
    assert_eq!(f3d.faces.len(), 2);
    assert!(!f3d.warnings.is_empty());
}

/// 谷折りで表裏判定が反転した利用者の展開図。テストはscratchpad等の外部fixtureを
/// 読まず、手順なしの角度操作を製品solverへ直接渡す。
fn valley_surface_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 1.0, 0.0),
            v(2, 1.0, 1.0),
            v(3, 0.0, 1.0),
            v(4, 0.0, 0.5),
            v(5, 1.0, 0.5),
            v(6, 0.5, 1.0),
            v(7, 0.5, 0.0),
            v(8, 0.5, 0.5),
            v(9, 0.792_893_218_813_452_5, 0.5),
            v(10, 0.5, 0.207_106_781_186_547_52),
            v(11, 0.25, 0.5),
            v(12, 0.5, 0.792_893_218_813_452_5),
            v(13, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            e(4, 3, 4, EdgeKind::Border),
            e(5, 4, 0, EdgeKind::Border),
            e(6, 1, 5, EdgeKind::Border),
            e(7, 5, 2, EdgeKind::Border),
            e(9, 2, 6, EdgeKind::Border),
            e(10, 6, 3, EdgeKind::Border),
            e(11, 0, 7, EdgeKind::Border),
            e(12, 7, 1, EdgeKind::Border),
            e(17, 0, 8, EdgeKind::Valley),
            e(18, 8, 2, EdgeKind::Valley),
            e(19, 8, 9, EdgeKind::Mountain),
            e(20, 9, 5, EdgeKind::Mountain),
            e(21, 2, 9, EdgeKind::Mountain),
            e(22, 9, 1, EdgeKind::Mountain),
            e(23, 8, 10, EdgeKind::Mountain),
            e(24, 10, 7, EdgeKind::Mountain),
            e(25, 0, 10, EdgeKind::Mountain),
            e(26, 10, 1, EdgeKind::Mountain),
            e(28, 11, 8, EdgeKind::Mountain),
            e(31, 6, 12, EdgeKind::Mountain),
            e(32, 12, 8, EdgeKind::Mountain),
            e(33, 2, 12, EdgeKind::Mountain),
            e(34, 12, 3, EdgeKind::Mountain),
            e(37, 4, 13, EdgeKind::Mountain),
            e(38, 13, 11, EdgeKind::Mountain),
            e(39, 0, 13, EdgeKind::Mountain),
            e(40, 13, 3, EdgeKind::Mountain),
            e(42, 13, 12, EdgeKind::Valley),
            e(43, 10, 9, EdgeKind::Valley),
        ],
        next_vertex_id: 14,
        next_edge_id: 44,
    }
}

fn valley_surface_drivers(angle: f64, center: Option<f64>) -> Vec<Driver> {
    let mut drivers: Vec<Driver> = [21, 22, 25, 26, 33, 34, 39, 40]
        .into_iter()
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: angle,
        })
        .collect();
    if let Some(target_angle_deg) = center {
        drivers.push(Driver {
            hinge: 43,
            target_angle_deg,
        });
    }
    drivers
}

/// A/B/C の各枝で全域木が受け取る角度を明示する。
///
/// ここで検査するのは solver の数値解ではなく、`cos(angle) < 0` を木に沿って
/// XORする規則である。値は90°の境界から十分離れた代表値に丸めてあり、
/// 浮動小数やプラットフォームによる solver の枝の差をgolden値にしない。
fn valley_surface_tree_angles(state: &str) -> HashMap<u32, f64> {
    let a = HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (20, -133.0),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -133.0),
        (25, 180.0),
        (31, -117.0),
        (32, 180.0),
        (34, 180.0),
        (37, -117.0),
        (39, 180.0),
        (40, 180.0),
        (42, -63.0),
        (43, -47.0),
    ]);
    match state {
        "A" => a,
        "B" => a.into_iter().map(|(edge, angle)| (edge, -angle)).collect(),
        "C" => HashMap::from([
            (17, -180.0),
            (18, -180.0),
            (20, -95.0),
            (21, 180.0),
            (22, 180.0),
            (23, 180.0),
            (24, -95.0),
            (25, 180.0),
            (31, -123.0),
            (32, 180.0),
            (34, 180.0),
            (37, -123.0),
            (39, 180.0),
            (40, 180.0),
            (42, -57.0),
            (43, -85.0),
        ]),
        _ => panic!("unknown valley-surface state {state}"),
    }
}

#[test]
fn valley_surface_explicit_angle_branches_carry_fold_parity() {
    let cp = valley_surface_cp();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 16);

    let expected = vec![
        (0, false),
        (1, true),
        (2, true),
        (3, false),
        (4, true),
        (5, false),
        (6, false),
        (7, true),
        (8, false),
        (9, true),
        (10, false),
        (11, true),
        (12, false),
        (13, false),
        (14, true),
        (15, true),
    ];
    for label in ["A", "B", "C"] {
        let frame = propagate(&cp, &faces, &valley_surface_tree_angles(label));
        let state: Vec<(u32, bool)> = to_frame3d(&cp, &faces, &frame)
            .faces
            .iter()
            .map(|face| (face.face, face.mirrored))
            .collect();
        assert_eq!(state, expected, "state {label} の面鏡映偶奇");
    }
}

#[test]
fn valley_surface_angle_only_states_return_finite_complete_frames() {
    let cp = valley_surface_cp();
    let faces = extract_faces(&cp);

    // 完全折り近傍のsolver解そのものは計算機ごとに枝が変わり得るため、
    // A/B/Cの操作が止まらず、全16面の有限な表示結果を返すことだけを固定する。
    for (label, drivers) in [
        ("A", valley_surface_drivers(180.0, None)),
        ("B", valley_surface_drivers(-180.0, None)),
        ("C", valley_surface_drivers(180.0, Some(-85.0))),
    ] {
        let result = solve_motion(&cp, &faces, &drivers, None, None, true).result;
        assert_eq!(result.frame.faces.len(), 16, "state {label}");
        assert!(
            result
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite()),
            "state {label} must return finite display coordinates"
        );
    }
}
