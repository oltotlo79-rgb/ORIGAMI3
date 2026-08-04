//! extract_faces のテスト。

use ori3_cp::faces::extract_faces;
use ori3_cp::graph::insert_segment;
use ori3_model::{CreasePattern, Document, EdgeKind, Paper, VertexId};

/// 正方形(輪郭4辺のみ)のCPを作る。
fn square_cp() -> CreasePattern {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
    .cp
}

/// 頂点列の符号付き面積(反時計回りで正)。
fn signed_area(cp: &CreasePattern, vs: &[VertexId]) -> f64 {
    let pos = |id: VertexId| {
        let v = cp.vertices.iter().find(|v| v.id == id).unwrap();
        (v.pos[0], v.pos[1])
    };
    let mut sum = 0.0;
    for i in 0..vs.len() {
        let (x0, y0) = pos(vs[i]);
        let (x1, y1) = pos(vs[(i + 1) % vs.len()]);
        sum += x0 * y1 - x1 * y0;
    }
    0.5 * sum
}

/// 米字(対角線2本+十字)のCPを作る。
fn kome_cp() -> CreasePattern {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
    cp
}

#[test]
fn square_only_is_one_face() {
    let cp = square_cp();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "外周面は除外され内部の面だけが残る");
    assert_eq!(faces[0].vertices.len(), 4);
    assert_eq!(faces[0].edges.len(), 4);
    assert!((signed_area(&cp, &faces[0].vertices) - 1.0).abs() < 1e-9);
}

#[test]
fn one_diagonal_makes_two_faces() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 2);
    let total: f64 = faces.iter().map(|f| signed_area(&cp, &f.vertices)).sum();
    assert!((total - 1.0).abs() < 1e-9, "面積の合計は紙全体と一致");
}

#[test]
fn kome_pattern_makes_eight_faces() {
    let cp = kome_cp();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 8);
    for f in &faces {
        let a = signed_area(&cp, &f.vertices);
        assert!((a - 0.125).abs() < 1e-9, "8つの三角形は各1/8の面積: {a}");
    }
}

#[test]
fn floating_segment_keeps_one_face() {
    let mut cp = square_cp();
    // 正方形内部に浮いた1本線(どの辺・頂点とも接続しない)
    insert_segment(&mut cp, [0.3, 0.5], [0.7, 0.5], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "ぶら下がり線は面を増やさない");
}

#[test]
fn dangling_slit_is_traversed_on_both_sides() {
    let mut cp = square_cp();
    // 下辺の中点から内部へ伸びるぶら下がり線(スリット)
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.5], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "スリットは面を増やさない");
    let f = &faces[0];
    let base = cp.vertices.iter().find(|v| v.pos == [0.5, 0.0]).unwrap().id;
    let tip = cp.vertices.iter().find(|v| v.pos == [0.5, 0.5]).unwrap().id;
    // 境界を両側から辿るため、付け根の頂点は2回・先端は1回現れる
    assert_eq!(f.vertices.iter().filter(|&&v| v == base).count(), 2);
    assert_eq!(f.vertices.iter().filter(|&&v| v == tip).count(), 1);
    // スリット辺は境界辺列に2回(往復で)現れる
    let slit = cp
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Mountain)
        .unwrap()
        .id;
    assert_eq!(f.edges.iter().filter(|&&e| e == slit).count(), 2);
    // 往復が相殺され、面積は紙全体のまま
    assert!((signed_area(&cp, &f.vertices) - 1.0).abs() < 1e-9);
}

#[test]
fn aux_edges_are_ignored() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Aux);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "補助線は面の境界に使わない");
}

#[test]
fn faces_are_counter_clockwise() {
    let cp = kome_cp();
    for f in extract_faces(&cp) {
        assert!(
            signed_area(&cp, &f.vertices) > 0.0,
            "面の頂点列は反時計回り: face {}",
            f.id
        );
    }
}

#[test]
fn extraction_is_deterministic() {
    let cp1 = kome_cp();
    let cp2 = kome_cp();
    let f1 = extract_faces(&cp1);
    let f2 = extract_faces(&cp2);
    assert_eq!(f1, f2, "同じ入力からは同じ順序・同じIDの面が得られる");
    // 面IDは0からの連番
    for (i, f) in f1.iter().enumerate() {
        assert_eq!(f.id, i as u32);
    }
    // 同じCPからの再抽出も一致
    assert_eq!(extract_faces(&cp1), f1);
}
