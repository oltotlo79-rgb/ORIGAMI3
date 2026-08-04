//! Task 1-7: 全域木の角度伝播のテスト。
//!
//! ヒンジ角の規約(ori3-rigid::tree のdocコメント参照):
//! 紙の表を+zとし、+θ=山折り(動く面が表から見て奥(−z)側へ畳まれる)、
//! −θ=谷折り(手前(+z)側へ畳まれる)。

use std::collections::HashMap;

use glam::{DMat3, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_model::{CreasePattern, EPS, Edge, EdgeKind, Vertex};
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
fn moved_polygon(cp: &CreasePattern, faces: &[Face], frame: &FoldedFrame, idx: usize) -> Vec<DVec3> {
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
