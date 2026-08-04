//! validate(CPの不変条件検査)のテスト。

use ori3_cp::graph::{insert_segment, move_vertex};
use ori3_cp::validate::validate;
use ori3_model::{CreasePattern, Document, Edge, EdgeKind, Paper, Vertex, VertexId};

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
fn floating_closed_loop_is_reported() {
    let mut cp = square_cp();
    // 外周と繋がらない三角形の閉ループ
    insert_segment(&mut cp, [0.3, 0.3], [0.7, 0.3], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.7, 0.3], [0.5, 0.7], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.5, 0.7], [0.3, 0.3], EdgeKind::Mountain);
    let warnings = validate(&cp);
    assert_eq!(warnings.len(), 1, "浮いた閉ループで警告1件: {warnings:?}");
    assert!(
        warnings[0].contains("分かれている"),
        "連結性の警告が出る: {}",
        warnings[0]
    );
}

#[test]
fn valid_kome_pattern_has_no_warnings() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
    assert!(validate(&cp).is_empty(), "正常な米字CPでは警告なし");
}

#[test]
fn crossing_after_move_vertex_is_reported() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Valley);
    assert!(validate(&cp).is_empty(), "移動前は警告なし");
    // 中央の頂点を紙の外へ動かすと、対角線の断片が右辺を跨ぐ
    let center = vertex_at(&cp, 0.5, 0.5).unwrap();
    move_vertex(&mut cp, center, [1.5, 0.5]);
    let warnings = validate(&cp);
    assert_eq!(
        warnings.len(),
        2,
        "対角線の断片2本が右辺と交差する: {warnings:?}"
    );
    assert!(warnings.iter().all(|w| w.contains("交差")));
}

#[test]
fn crushed_bridge_edge_is_reported_as_degenerate_and_disconnected() {
    let mut cp = square_cp();
    // 下辺から内部へ伸びる橋辺と、その先端に付く三角形ループ
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.2], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.5, 0.2], [0.7, 0.5], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.7, 0.5], [0.3, 0.5], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.3, 0.5], [0.5, 0.2], EdgeKind::Mountain);
    assert!(validate(&cp).is_empty(), "移動前は警告なし");
    // 先端を付け根の真上まで動かして橋辺を長さゼロに潰す。
    // 頂点IDのグラフとしては繋がったままだが、面抽出視点ではループが浮く
    let tip = vertex_at(&cp, 0.5, 0.2).unwrap();
    move_vertex(&mut cp, tip, [0.5, 0.0]);
    let warnings = validate(&cp);
    assert_eq!(warnings.len(), 2, "退化辺と連結性の警告: {warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("ほぼゼロ")));
    assert!(warnings.iter().any(|w| w.contains("分かれている")));
}

#[test]
fn degenerate_edge_is_reported() {
    // v0==v1 の退化辺を直接組んだCP
    let cp = CreasePattern {
        vertices: vec![
            Vertex {
                id: 0,
                pos: [0.0, 0.0],
            },
            Vertex {
                id: 1,
                pos: [1.0, 0.0],
            },
        ],
        edges: vec![
            Edge {
                id: 0,
                v0: 0,
                v1: 1,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 1,
                v0: 0,
                v1: 0,
                kind: EdgeKind::Mountain,
            },
        ],
        next_vertex_id: 2,
        next_edge_id: 2,
    };
    let warnings = validate(&cp);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("ほぼゼロ"), "{}", warnings[0]);
    assert!(warnings[0].contains("辺1"));
}

#[test]
fn edge_with_missing_vertex_is_reported() {
    // 存在しない頂点ID(99)を参照する辺を直接組んだCP
    let cp = CreasePattern {
        vertices: vec![Vertex {
            id: 0,
            pos: [0.0, 0.0],
        }],
        edges: vec![Edge {
            id: 0,
            v0: 0,
            v1: 99,
            kind: EdgeKind::Mountain,
        }],
        next_vertex_id: 1,
        next_edge_id: 1,
    };
    let warnings = validate(&cp);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("存在しない"), "{}", warnings[0]);
    assert!(warnings[0].contains("辺0"));
}

#[test]
fn overlap_after_move_vertex_is_reported() {
    let mut cp = square_cp();
    // 下辺の中点から上へ伸びる線
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.5], EdgeKind::Mountain);
    let tip = vertex_at(&cp, 0.5, 0.5).unwrap();
    // 先端を下辺上へ動かすと、線が下辺の断片と同一直線上で重なる
    move_vertex(&mut cp, tip, [0.8, 0.0]);
    let warnings = validate(&cp);
    assert!(
        warnings.iter().any(|w| w.contains("重なっている")),
        "重なりの警告が出る: {warnings:?}"
    );
}
