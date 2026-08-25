use ori3_export::fold::{
    FoldComparisonOptions, canonicalize_fold_1_2, compare_fold_1_2, parse_fold_1_2,
};

const MINIMAL: &str = include_str!("fixtures/fold/minimal-supported.fold");
const MINIMAL_CANONICAL: &str = include_str!("fixtures/fold/minimal-supported.canonical.json");

#[test]
fn canonical_json_is_compact_deterministic_and_readable() {
    let file = parse_fold_1_2(MINIMAL).expect("手書きの限定profileをparseできる");
    let first = canonicalize_fold_1_2(&file).expect("canonical JSONを作れる");
    let second = canonicalize_fold_1_2(&file).expect("同じ入力を再度canonical化できる");

    assert_eq!(first, second, "同じtyped FOLDから同じ文字列を作る");
    assert_eq!(
        first,
        MINIMAL_CANONICAL.trim_end(),
        "通常検査は追跡済みcanonical fixtureを読み取って照合する"
    );
    assert!(!first.contains('\n'), "canonical JSONは空白差を持たない");
    assert!(
        first.find("\"edges_assignment\"") < first.find("\"file_spec\""),
        "object keyを辞書順へ固定する: {first}"
    );
    parse_fold_1_2(&first).expect("canonical JSONをtyped parserで読み直せる");
}

#[test]
fn comparison_uses_measured_epsilon_only_for_coordinates_and_angles() {
    let original = parse_fold_1_2(MINIMAL).expect("基準fixtureをparseできる");
    let mut near = original.clone();
    let near_vertices = near.root.vertices_coords.as_mut().expect("頂点座標がある");
    near_vertices[0][0] += 5e-10;
    near_vertices[3][0] += 5e-10;
    near.root.edges_fold_angle.as_mut().expect("角度がある")[4] = Some(-90.0 + 5e-10);

    // §12.6の境界は座標・角度とも1e-9。実測境界そのものではなく、その半分の
    // 5e-10を丸め差として許し、2倍の2e-9は別物として検出する。
    let near_comparison = compare_fold_1_2(&original, &near, FoldComparisonOptions::default())
        .expect("profile内の2入力を比較できる");
    assert!(
        near_comparison.is_equivalent(),
        "許容差内だけなら同値: {:?}",
        near_comparison.differences
    );

    let mut far = original.clone();
    let far_vertices = far.root.vertices_coords.as_mut().expect("頂点座標がある");
    far_vertices[0][0] += 2e-9;
    far_vertices[3][0] += 2e-9;
    let far_comparison = compare_fold_1_2(&original, &far, FoldComparisonOptions::default())
        .expect("profile内の2入力を比較できる");
    assert!(
        far_comparison
            .differences
            .iter()
            .any(|difference| difference.path == "$.vertices_coords[0][0]"),
        "許容差を超えた座標pathを示す: {:?}",
        far_comparison.differences
    );
}

#[test]
fn face_order_direction_is_canonical_but_topology_index_order_is_exact() {
    let original = parse_fold_1_2(MINIMAL).expect("基準fixtureをparseできる");
    let mut same_order = original.clone();
    same_order.root.face_orders = Some(vec![vec![1, 0, -1]]);
    let order_comparison =
        compare_fold_1_2(&original, &same_order, FoldComparisonOptions::default())
            .expect("faceOrdersを比較できる");
    assert!(
        order_comparison.is_equivalent(),
        "同じcanonical triple集合は同値"
    );

    let mut reordered = original.clone();
    reordered
        .root
        .edges_vertices
        .as_mut()
        .expect("辺がある")
        .swap(0, 1);
    reordered
        .root
        .edges_assignment
        .as_mut()
        .expect("assignmentがある")
        .swap(0, 1);
    reordered
        .root
        .edges_fold_angle
        .as_mut()
        .expect("角度がある")
        .swap(0, 1);
    let topology_comparison =
        compare_fold_1_2(&original, &reordered, FoldComparisonOptions::default())
            .expect("topologyを比較できる");
    assert!(
        topology_comparison
            .differences
            .iter()
            .any(|difference| difference.path.starts_with("$.edges_vertices[")),
        "index順を含むtopologyはexact: {:?}",
        topology_comparison.differences
    );
}

#[test]
fn invalid_epsilon_is_reported_without_panicking() {
    let file = parse_fold_1_2(MINIMAL).expect("基準fixtureをparseできる");
    let comparison = compare_fold_1_2(
        &file,
        &file,
        FoldComparisonOptions {
            coordinate_epsilon: f64::NAN,
            angle_epsilon_deg: 1e-9,
        },
    )
    .expect("設定誤りもpanicせず比較結果へ返す");
    assert_eq!(comparison.differences.len(), 1);
    assert_eq!(comparison.differences[0].path, "$");
}
