//! 線が密に交わる展開図での、細分・面の抽出・点検の受け入れテスト。
//!
//! `graph.rs` の交差細分は最大2本(5頂点・8辺)まで、`faces.rs` の補助線の検査は
//! 補助線1本だけ、`validate` と `local_violations` は別々のファイルで別々の展開図に
//! 掛けている。そのため次の3つが、この規模では一度も確かめられていなかった。
//!
//! 1. 多数の線が互いに交わったときの厳密な細分(頂点数・辺数)
//! 2. 補助線どうしが密に交差しても、面の境界には使われないこと
//! 3. 同じ1つの展開図で、`local_violations` と `validate` が両方とも空になること
//!
//! ここで使う22本は、格子・対角線・`Q(√3)` の点を混ぜた**作品とは無関係の**模様である。
//! `2 - √3`(= tan15°)と `√3 - 1` を含めて、無理数の座標でも細分が壊れないことを見る。

use ori3_cp::{extract_faces, insert_segment, local_violations, validate};
use ori3_model::{CreasePattern, Document, EdgeKind, Paper};

/// 互いに交わる22本の線。端点はすべて紙の輪郭上に置く。
///
/// 内訳: 対角2 / 縦3 / 横3 / 斜め4 / `Q(√3)` の6 / 四隅を結ぶ4。
fn crossing_lines() -> [([f64; 2], [f64; 2]); 22] {
    let root3 = 3.0_f64.sqrt();
    // tan15° = 2 - √3 ≈ 0.267949、tan60° - 1 の代わりに √3 - 1 ≈ 0.732051 を使う。
    let h = 2.0 - root3;
    let g = root3 - 1.0;
    let third = 1.0 / 3.0;
    let twothird = 2.0 / 3.0;
    [
        // 対角
        ([0.0, 0.0], [1.0, 1.0]),
        ([1.0, 0.0], [0.0, 1.0]),
        // 縦
        ([0.25, 0.0], [0.25, 1.0]),
        ([0.5, 0.0], [0.5, 1.0]),
        ([0.75, 0.0], [0.75, 1.0]),
        // 横
        ([0.0, 0.25], [1.0, 0.25]),
        ([0.0, 0.5], [1.0, 0.5]),
        ([0.0, 0.75], [1.0, 0.75]),
        // 三等分点を結ぶ斜め
        ([0.0, third], [1.0, twothird]),
        ([0.0, twothird], [1.0, third]),
        ([third, 0.0], [twothird, 1.0]),
        ([twothird, 0.0], [third, 1.0]),
        // Q(√3) の点を通る線
        ([0.0, 0.0], [1.0, h]),
        ([0.0, 0.0], [h, 1.0]),
        ([1.0, 1.0], [0.0, 1.0 - h]),
        ([1.0, 1.0], [1.0 - h, 0.0]),
        ([0.0, g], [g, 0.0]),
        ([1.0, 1.0 - g], [1.0 - g, 1.0]),
        // 四隅寄りを結ぶ
        ([0.0, 0.25], [0.75, 1.0]),
        ([0.25, 0.0], [1.0, 0.75]),
        ([0.0, 0.75], [0.75, 0.0]),
        ([0.25, 1.0], [1.0, 0.25]),
    ]
}

/// 22本のうち先頭1本だけを谷折り、残り21本を補助線として引いた展開図。
fn dense_cp() -> CreasePattern {
    let mut document = Document::new(Paper {
        width_mm: 200.0,
        height_mm: 200.0,
    });
    for (index, (a, b)) in crossing_lines().into_iter().enumerate() {
        let kind = if index == 0 {
            EdgeKind::Valley
        } else {
            EdgeKind::Aux
        };
        insert_segment(&mut document.cp, a, b, kind);
    }
    document.cp
}

/// 22本すべてを谷折りとして引いた展開図。
fn dense_cp_all_creases() -> CreasePattern {
    let mut document = Document::new(Paper {
        width_mm: 200.0,
        height_mm: 200.0,
    });
    for (a, b) in crossing_lines() {
        insert_segment(&mut document.cp, a, b, EdgeKind::Valley);
    }
    document.cp
}

#[test]
fn many_crossing_lines_subdivide_into_an_exact_number_of_vertices_and_edges() {
    let cp = dense_cp();

    // 実測値(このテストを手元で実行して数えた): 線22本 -> 頂点129 / 辺272。
    // 交点の数は整数の組み合わせだけで決まるので、計算機が変わっても同じになる。
    // 既存の `graph.rs` の交差細分は最大2本(頂点5 / 辺8)までしか見ていない。
    assert_eq!(cp.vertices.len(), EXPECTED_VERTICES);
    assert_eq!(cp.edges.len(), EXPECTED_EDGES);

    // 線種の順序を入れ替えても同じ細分になる(挿入順に依存しない)。
    let all_creases = dense_cp_all_creases();
    assert_eq!(all_creases.vertices.len(), cp.vertices.len());
    assert_eq!(all_creases.edges.len(), cp.edges.len());

    // 同じ手順をもう一度たどっても、1つも増減しない(決定性)。
    let again = dense_cp();
    assert_eq!(again.vertices.len(), cp.vertices.len());
    assert_eq!(again.edges.len(), cp.edges.len());

    // 無理数の座標(2 - √3, √3 - 1)を端点にした線が、実際に頂点として残っている。
    let root3 = 3.0_f64.sqrt();
    for point in [[0.0, root3 - 1.0], [1.0, 2.0 - root3]] {
        assert!(
            cp.vertices
                .iter()
                .any(|vertex| (vertex.pos[0] - point[0]).abs() < 1e-9
                    && (vertex.pos[1] - point[1]).abs() < 1e-9),
            "無理数の端点 {point:?} が頂点として残っていない"
        );
    }

    println!(
        "密な交差: 線22本 -> 頂点{} / 辺{} / 面{}",
        cp.vertices.len(),
        cp.edges.len(),
        extract_faces(&cp).len()
    );
}

#[test]
fn crossing_auxiliary_lines_never_bound_a_face() {
    let cp = dense_cp();

    // 補助線21本が互いに何度も交差していても、面を割るのは谷折り1本(対角線)だけ。
    assert_eq!(
        extract_faces(&cp).len(),
        2,
        "補助線どうしの交差は面の境界に使わない"
    );

    // 同じ22本を全部折り目にすると、今度は面が増える。
    // 「補助線だから割らない」のであって「線が届いていない」のではないことを示す。
    let all_creases = dense_cp_all_creases();
    assert_eq!(all_creases.vertices.len(), cp.vertices.len());
    assert!(
        extract_faces(&all_creases).len() > 2,
        "全部を折り目にすれば面は増える"
    );

    println!(
        "補助線21本の面数={} / 全部折り目にした面数={}",
        extract_faces(&cp).len(),
        extract_faces(&all_creases).len()
    );
}

#[test]
fn the_same_dense_pattern_passes_both_checks_at_once() {
    let cp = dense_cp();

    // 同じ1つの展開図に、2つの点検を両方掛ける。
    // 片方だけを見ていると、一方が拾う不整合をもう一方が作っていても気づけない。
    assert!(
        local_violations(&cp).is_empty(),
        "平らに折りにくい点={:?}",
        local_violations(&cp)
    );
    assert!(validate(&cp).is_empty(), "CP検証警告={:?}", validate(&cp));

    let all_creases = dense_cp_all_creases();
    assert!(
        validate(&all_creases).is_empty(),
        "全部折り目のCP検証警告={:?}",
        validate(&all_creases)
    );
}

/// 22本を引き終えたときの頂点数(実測)。輪郭の4隅 + 端点 + 交点。
const EXPECTED_VERTICES: usize = 129;
/// 22本を引き終えたときの辺数(実測)。
const EXPECTED_EDGES: usize = 272;
