// 面対の並びまで含んでいたが、重なっていない2枚の上下は紙の重なり方を表さない。

#[test]
fn surface_order_179_999_to_180_all_110_creases() {
    let diagrams = boundary_diagrams();
    let nudged_diagrams = diagrams.iter().map(one_ulp_nudged).collect::<Vec<_>>();
    let mut total_hinges = 0;
    let mut robust_stacks = 0_usize;
    let mut changed_hinges = BTreeSet::<(&'static str, EdgeId)>::new();
    let mut changed_directions = 0;
    let mut flipped_hinges = BTreeSet::<(&'static str, EdgeId)>::new();
    let mut flipped_directions = 0;
    for (diagram, nudged) in diagrams.iter().zip(&nudged_diagrams) {
        let mut diagram_changed = BTreeSet::new();
        let mut diagram_flipped = BTreeSet::new();
        total_hinges += diagram.hinges.len();
        for &(hinge, kind) in &diagram.hinges {
            for sign in [1.0, -1.0] {
                let ladder = boundary_ladder(diagram, hinge, sign);
                let stacks = rounding_robust_stacks(diagram, &ladder, nudged, hinge, sign);
                robust_stacks += stacks.len();
                let after_state = &ladder[ladder.len() - 1].1;
                let before_state = &ladder[ladder.len() - 2].1;
                let flipped = stacks_that_flip_between_endpoints(
                    &before_state.frame,
                    &after_state.frame,
                    &stacks,
                );
                if !flipped.is_empty() {
                    flipped_hinges.insert((diagram.name, hinge));
                    flipped_directions += 1;
                    diagram_flipped.insert(hinge);
                    println!(
                        "SURFACE_180_RANK_CHANGE diagram={} edge={} kind={kind:?} direction={sign:+} robust_stacks={} flipped={flipped:?}",
                        diagram.name,
                        hinge,
                        stacks.len(),
                    );
                }
                let view = camera(diagram.paper_width, diagram.paper_height, 1.0);
                let before = visual_image(diagram, &before_state.frame, VIEWPORT, view);
                let after = visual_image(diagram, &after_state.frame, VIEWPORT, view);
                if before.visible_back_faces != after.visible_back_faces {
                    let difference = before.difference(&after, VIEWPORT);
                    let max_vertex_distance =
                        max_vertex_distance(&before_state.frame, &after_state.frame);
                    let before_exact = before_state
                        .angles
                        .values()
                        .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-6)
                        .count();
                    let after_exact = after_state
                        .angles
                        .values()
                        .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-6)
                        .count();
                    changed_directions += 1;
                    changed_hinges.insert((diagram.name, hinge));
                    diagram_changed.insert(hinge);
                    println!(
                        "SURFACE_180_CHANGE diagram={} edge={} kind={kind:?} direction={sign:+} before_back={} after_back={} owner_pixels={} color_pixels={} coverage_pixels={} side_pixels={} face_only_pixels={} color_bounds={:?} max_vertex_distance={max_vertex_distance:.9e} before_exact={} after_exact={}",
                        diagram.name,
                        hinge,
                        ids(&before.visible_back_faces),
                        ids(&after.visible_back_faces),
                        difference.owner_pixels,
                        difference.color_pixels,
                        difference.coverage_pixels,
                        difference.side_pixels,
                        difference.face_only_pixels,
                        difference.color_bounds,
                        before_exact,
                        after_exact,
                    );
                }
            }
        }
        println!(
            "SURFACE_180_DIAGRAM diagram={} hinges={} changed_hinges={} changed_ids={}",
            diagram.name,
            diagram.hinges.len(),
            diagram_changed.len(),
            ids(&diagram_changed),
        );
        println!(
            "SURFACE_180_RANK_DIAGRAM diagram={} hinges={} changed_hinges={} changed_ids={}",
            diagram.name,
            diagram.hinges.len(),
            diagram_flipped.len(),
            ids(&diagram_flipped),
        );
    }
    println!(
        "SURFACE_180_TOTAL diagrams={} hinges={} directions={} changed_hinges={} changed_directions={}",
        diagrams.len(),
        total_hinges,
        total_hinges * 2,
        changed_hinges.len(),
        changed_directions,
    );
    println!(
        "SURFACE_180_RANK_TOTAL robust_stacks={robust_stacks} changed_hinges={} changed_directions={flipped_directions} changed_edges={flipped_hinges:?}",
        flipped_hinges.len(),
    );
    assert_eq!(total_hinges, 110);
    assert!(
        flipped_hinges.is_empty(),
        "179.999 and 180 degrees must stack the paper the same way for every pair whose stacking the geometry determines and one ULP of input does not change: {flipped_hinges:?}"
    );
    // 主張の対象が空になっていないことの下限。梯子や1 ULPの選別が壊れて対象が
    // 消えると、この検査は何も主張しないまま緑になってしまう。
    //
    // 実測: `float_roundtrip` あり **4888組**、なし **4910組**(どちらもこの
    // 計算機。梯子で決まった 4936組のうち、1 ULPで答えが変わった 48組 / 26組を
    // 外した数)。計算機が変わると外れる組はもっと増え得るので、下限は
    // 「空回りかどうか」だけが分かる 4,000組に置く。実際に空回りすれば0に近い
    // 値まで落ちるので、これで検知できる。
    assert!(
        robust_stacks >= 4_000,
        "the measured stacking must still cover the paper: robust_stacks={robust_stacks}"
    );
    assert!(
        changed_hinges.len() < 79,
        "stage C must reduce the 79 previously measured endpoint changes: {changed_hinges:?}"
    );
}

// 以前に端点で重なり順が変わっていた19本について、完全に折った姿勢でも
// **紙の重なり方**が変わらないことを検査する。
//
// **以前は `assert_eq!(surface_rank_order(&after.frame), expected)` の形で、
// 2回のsolveの結果の並びをそのまま比べていた。** 完全に折った状態のすぐ近くでは、
// 解が近くの別の折り方へ移るだけで並びが入れ替わるので、この形は計算機や丸めの
// 違いで落ち得る(CLAUDE.md §10.7.7 が禁じる「solveの結果に期待値を結び付けた
// 検査」)。同じ形だった `surface_order_179_999_to_180_all_110_creases` は、
// 作品ファイルの小数を正確に読む `serde_json` の `float_roundtrip` を入れただけで
// 実際に落ちた(`folded-sample.ori3` の辺306、面31と面34。隙間 −1.902e-6 が
// 入力1 ULPで +3.295e-5 へ符号ごと変わる)。**こちらが落ちていなかったのは、
// 辺306がこの19本に入っていなかったからにすぎない。**
//
// そこで、110本の掃引と同じ3段の形へ書き直した。主張は弱めていない。
//
// 1. 180°の手前の4段(179.5 / 179.9 / 179.99 / 179.999)は面どうしが実際に
//    離れているので、**その姿勢そのものから**面対の上下を測る(`determined_stacks`)。
// 2. **入力の座標を1 ULP動かした複製**でも同じ梯子を作り、測った上下が変わらない
//    面対だけを主張の対象にする(`rounding_robust_stacks`)。
// 3. 対象の面対について、次の3つの姿勢が**同じ紙の重なり方**を表していることを
//    主張する。上下は正準法線ではなく姿勢によらない**巻き方向**で読むので、
//    軸の反転では入れ替わらない(`winding_sign`)。
//    - 179.999°の姿勢と180°の姿勢
//    - 180°の姿勢と、そこから同じ180°へ解き直した姿勢
