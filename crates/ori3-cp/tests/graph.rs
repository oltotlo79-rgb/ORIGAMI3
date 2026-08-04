//! insert_segment / remove_edges / move_vertex のテスト。

use ori3_cp::graph::{insert_segment, move_vertex, remove_edges};
use ori3_model::{CreasePattern, Document, EdgeKind, Paper, VertexId};

/// 正方形(輪郭4辺のみ)のCPを作る。
fn square_cp() -> CreasePattern {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
    .cp
}

/// 指定座標(許容1e-9)にある頂点を探す。
fn vertex_at(cp: &CreasePattern, x: f64, y: f64) -> Option<VertexId> {
    cp.vertices
        .iter()
        .find(|v| (v.pos[0] - x).abs() <= 1e-9 && (v.pos[1] - y).abs() <= 1e-9)
        .map(|v| v.id)
}

#[test]
fn diagonal_snaps_to_corners_and_adds_one_edge() {
    let mut cp = square_cp();
    let added = insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    assert_eq!(added.len(), 1);
    assert_eq!(cp.vertices.len(), 4, "端点は既存の角に吸着し頂点は増えない");
    assert_eq!(cp.edges.len(), 5);
    let e = cp.edges.iter().find(|e| e.id == added[0]).unwrap();
    assert_eq!(e.kind, EdgeKind::Valley);
    let corner0 = vertex_at(&cp, 0.0, 0.0).unwrap();
    let corner2 = vertex_at(&cp, 1.0, 1.0).unwrap();
    assert!(
        (e.v0 == corner0 && e.v1 == corner2) || (e.v0 == corner2 && e.v1 == corner0),
        "対角線は既存の角同士を結ぶ"
    );
}

#[test]
fn crossing_diagonals_are_split_at_intersection() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    let added = insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Mountain);
    assert_eq!(added.len(), 2, "2本目は交点で2辺に分かれる");
    assert_eq!(cp.vertices.len(), 5);
    assert_eq!(cp.edges.len(), 8);
    let center = vertex_at(&cp, 0.5, 0.5).expect("交点(0.5,0.5)に頂点ができる");
    // 1本目の対角線も交点で分割され、kindはValleyのまま保たれる
    let valley_halves: Vec<_> = cp
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Valley)
        .collect();
    assert_eq!(valley_halves.len(), 2);
    assert!(
        valley_halves
            .iter()
            .all(|e| e.v0 == center || e.v1 == center)
    );
}

#[test]
fn duplicate_segment_is_noop() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    let before = cp.clone();
    let added = insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    assert!(added.is_empty(), "既存線と同一の線分は何も追加しない");
    assert_eq!(cp, before, "CPは完全に変化なし(ID採番も含む)");
}

#[test]
fn endpoint_on_edge_interior_splits_the_edge() {
    let mut cp = square_cp();
    // 下辺と上辺の中点を結ぶ縦線。両端点が既存辺の途中に乗る
    let added = insert_segment(&mut cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
    assert_eq!(added.len(), 1);
    assert_eq!(cp.vertices.len(), 6, "下辺・上辺の中点に頂点ができる");
    assert_eq!(cp.edges.len(), 7, "下辺2+上辺2+左右2+縦線1");
    assert!(vertex_at(&cp, 0.5, 0.0).is_some());
    assert!(vertex_at(&cp, 0.5, 1.0).is_some());
    // 分割された輪郭辺のkindはBorderのまま
    assert_eq!(
        cp.edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Border)
            .count(),
        6
    );
}

#[test]
fn overlap_with_existing_edge_keeps_existing_kind() {
    let mut cp = square_cp();
    // 下辺の左半分に重なる線分をMountainで挿入しても、既存Borderを上書きしない
    let added = insert_segment(&mut cp, [0.0, 0.0], [0.5, 0.0], EdgeKind::Mountain);
    assert!(added.is_empty(), "重なり区間には新しい辺を作らない");
    assert_eq!(
        cp.vertices.len(),
        5,
        "重なりの端(0.5,0)で既存辺が分割される"
    );
    assert_eq!(cp.edges.len(), 5);
    assert!(
        cp.edges.iter().all(|e| e.kind == EdgeKind::Border),
        "既存辺のkindはBorderのまま"
    );
}

#[test]
fn endpoint_within_eps_snaps_to_vertex() {
    let mut cp = square_cp();
    let added = insert_segment(&mut cp, [1e-10, -1e-10], [1.0, 1.0], EdgeKind::Valley);
    assert_eq!(added.len(), 1);
    assert_eq!(cp.vertices.len(), 4, "EPS以内の端点は既存頂点へ吸着");
}

#[test]
fn remove_edges_cleans_isolated_vertices() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Mountain);
    let inner: Vec<_> = cp
        .edges
        .iter()
        .filter(|e| e.kind != EdgeKind::Border)
        .map(|e| e.id)
        .collect();
    remove_edges(&mut cp, &inner);
    assert_eq!(cp.edges.len(), 4, "輪郭4辺だけが残る");
    assert_eq!(cp.vertices.len(), 4, "中央の孤立頂点は掃除される");
}

#[test]
fn move_vertex_updates_position_only() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Mountain);
    let center = vertex_at(&cp, 0.5, 0.5).unwrap();
    let edges_before = cp.edges.clone();
    move_vertex(&mut cp, center, [0.4, 0.55]);
    let v = cp.vertices.iter().find(|v| v.id == center).unwrap();
    assert_eq!(v.pos, [0.4, 0.55]);
    assert_eq!(cp.edges, edges_before, "辺の接続やkindは変わらない");
}

#[test]
fn insert_segment_tolerates_dangling_reference_edge() {
    let mut cp = square_cp();
    // 存在しない頂点を参照する辺(参照切れ)を直接混入させる
    cp.edges.push(ori3_model::Edge {
        id: cp.next_edge_id,
        v0: 0,
        v1: 999,
        kind: EdgeKind::Mountain,
    });
    cp.next_edge_id += 1;
    // 参照切れ辺があってもpanicせず、通常どおり挿入できる
    let added = insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    assert_eq!(added.len(), 1);
}
