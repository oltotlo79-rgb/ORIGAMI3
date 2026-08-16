//! 作業20「完成の目標の4指標」の検査。
//!
//! 利用者が指定するのは **角の数・長さ・太さ・位置**。折り順を探すときに
//! 「いまの形が完成形へどれだけ近づいたか」を測る4つの物差しが、
//! 次を満たすことを確かめる。
//!
//! | # | 確かめること |
//! |---|---|
//! | 1 | 葉1〜12本の12通りで4つとも数になる(無限大・NaNが0件) |
//! | 2 | 完成形そのものを入れると4つとも最良(= 0.0)になる |
//! | 3 | わざと崩すと、対応する物差しが悪化する |
//! | 4 | 同じ入力を10回測って同じ値になる |
//! | 5 | 先端と紙の場所の対応は作業9の記録をそのまま使い、推測しない |
//!
//! **折り順の探索はここでは作らない**(作業21・22)。4つをどう合わせて1つの
//! 順位にするかも決めない(作業22)。

use ori3_propose::skeleton::{Skeleton, SkeletonNode, TipPos2d};
use ori3_propose::{
    FinishGaps, FinishTarget, FinishedForm, MeasuredTip, POSITION_GAP_MAX, Packing, count_gap,
    finish_gaps, generate, length_gap, pack, position_gap, width_gap,
};

/// 検査で使う紙の大きさ。
const PAPER: (f64, f64) = (1.0, 1.0);
/// 配置の乱数の種とやり直しの回数。既存の検査と同じ値。
const SEED: u64 = 2026;
const STARTS: usize = 8;

/// 角を `n` 本ぶら下げた星形の骨格。長さ・太さ・位置を1本ずつ変える。
///
/// - 長さは `0.6 + 0.03 × 番号`、太さは `0.8 + 0.02 × 番号` で、
///   本ごとに違う値にして「1本だけ崩す」検査が効くようにする。
/// - 位置は原点を囲む半径 `0.9` の円周上へ等間隔に置く(枠は -1.0〜1.0)。
///   枠いっぱいに広げないのは、指定が枠の縁に触れていなくても測れることを
///   同時に確かめるため。
fn star_with_positions(n: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        let k = f64::from(i);
        let angle = std::f64::consts::TAU * f64::from(i - 1) / f64::from(n);
        let mut node = SkeletonNode::new(i, Some(0), 0.6 + 0.03 * k);
        node.width_factor = 0.8 + 0.02 * k;
        nodes.push(node.with_tip_pos(TipPos2d::new(0.9 * angle.cos(), 0.9 * angle.sin())));
    }
    Skeleton { nodes }
}

/// 骨格から提案を1件作る。
fn propose(skeleton: &Skeleton) -> (Packing, ori3_propose::ProposalResult) {
    let candidates = pack(skeleton, PAPER.0, PAPER.1, SEED, STARTS);
    assert!(!candidates.is_empty(), "配置に失敗した");
    let result = generate(skeleton, &candidates[0], PAPER.0, PAPER.1).expect("生成に失敗した");
    (candidates[0].clone(), result)
}

/// 「折り上げた紙で測った先端の点」を、指定どおりに折れた場合として作る。
///
/// 完成形の位置は紙を折り上げないと決まらないので、この検査では
/// 「指定どおりの位置に来た」という点の並びを作って渡す。単位が変わっても
/// 同じ値になることを見るため、胴の中心を `(3.0, -2.0)`、大きさを40倍にしてある。
fn tip_points_as_specified(target: &FinishTarget) -> ([f64; 2], Vec<(u32, [f64; 2])>) {
    let center = [3.0, -2.0];
    let points = target
        .tips
        .iter()
        .filter_map(|t| {
            t.pos
                .map(|p| (t.leaf_id, [center[0] + p.x * 40.0, center[1] + p.y * 40.0]))
        })
        .collect();
    (center, points)
}

// ---------------------------------------------------------------------------
// 合格条件2: 葉1〜12本の12通りで、4つとも数になる
// ---------------------------------------------------------------------------

/// 実測(このファイルの出力): 12通りすべてで数・長さ・太さ・位置が有限。
/// 提案から測った形では位置が未測定になるため位置は 1.0(いちばん遠い扱い)、
/// 折り上げた点を入れた形では 4つとも 0.0 になる。
#[test]
fn the_four_gaps_are_finite_for_one_to_twelve_leaves() {
    println!("\n### 葉1〜12本での4指標\n");
    println!("| 角の本数 | 数 | 長さ | 太さ | 位置(未測定) | 位置(測定あり) |");
    println!("|---:|---:|---:|---:|---:|---:|");
    let mut finite_cases = 0;
    for n in 1..=12u32 {
        let skeleton = star_with_positions(n);
        let (packing, result) = propose(&skeleton);
        let target = FinishTarget::from_skeleton(&skeleton);
        let form = FinishedForm::from_proposal(&skeleton, &packing, &result);

        assert_eq!(
            form.tips.len(),
            n as usize,
            "角{n}本: 測った先端の数が指定と違う"
        );
        for t in &target.tips {
            let hits = form.tips.iter().filter(|m| m.leaf_id == t.leaf_id).count();
            assert_eq!(hits, 1, "角{n}本: 葉{}の測定点が1件でない", t.leaf_id);
        }

        let bare = finish_gaps(&target, &form);
        let (center, points) = tip_points_as_specified(&target);
        let posed = form.with_tip_points(&target, center, &points);
        let full = finish_gaps(&target, &posed);

        for (label, g) in [("位置なし", bare), ("位置あり", full)] {
            assert!(
                g.all_finite(),
                "角{n}本({label}): 数にならない値が出た: {g:?}"
            );
            for v in [g.count, g.length, g.width, g.position] {
                assert!(v >= 0.0, "角{n}本({label}): 負の隔たり {v}");
            }
        }
        finite_cases += 1;

        println!(
            "| {n} | {:.6} | {:.6} | {:.6} | {:.6} | {:.6} |",
            bare.count, bare.length, bare.width, bare.position, full.position
        );
    }
    assert_eq!(finite_cases, 12, "12通りすべてを測れていない");
}

// ---------------------------------------------------------------------------
// 合格条件3: 完成形そのものを入れると4つとも最良になる
// ---------------------------------------------------------------------------

/// 「最良」は **4つとも 0.0**(隔たりが無い)と定める。
///
/// 実測: 葉1〜12本の12通りすべてで、4つとも `0.0` と完全一致した。
#[test]
fn the_finished_form_itself_scores_best_on_all_four() {
    for n in 1..=12u32 {
        let skeleton = star_with_positions(n);
        let target = FinishTarget::from_skeleton(&skeleton);
        let form = FinishedForm::matching(&target);
        let gaps = finish_gaps(&target, &form);
        assert_eq!(
            gaps,
            FinishGaps::BEST,
            "角{n}本: 完成形そのものなのに最良にならなかった"
        );
    }
}

/// 提案から測った形でも、指定どおりに折り上がった点を入れれば最良になる。
///
/// 提案から測った形は、紙の上の割り算と枠のそろえ方で最下位の桁だけがずれうる
/// (折り上げた点は40倍の大きさ・別の場所で測った値として渡している)。そこで
/// 完全一致ではなく `1e-12` 未満で見る。
///
/// 実測(12通りの最大): 数 `0`、長さ `6.37e-17`、太さ `8.81e-17`、位置 `3.03e-17`。
/// 上限 `1e-12` に対して4桁以上の余裕がある。
#[test]
fn a_proposal_that_reaches_the_specification_scores_best() {
    let mut worst = FinishGaps::BEST;
    for n in 1..=12u32 {
        let skeleton = star_with_positions(n);
        let (packing, result) = propose(&skeleton);
        let target = FinishTarget::from_skeleton(&skeleton);
        let (center, points) = tip_points_as_specified(&target);
        let form = FinishedForm::from_proposal(&skeleton, &packing, &result)
            .with_tip_points(&target, center, &points);
        let gaps = finish_gaps(&target, &form);
        for (label, v) in [
            ("数", gaps.count),
            ("長さ", gaps.length),
            ("太さ", gaps.width),
            ("位置", gaps.position),
        ] {
            assert!(v < 1e-12, "角{n}本: {label}が最良から離れている: {gaps:?}");
        }
        worst.count = worst.count.max(gaps.count);
        worst.length = worst.length.max(gaps.length);
        worst.width = worst.width.max(gaps.width);
        worst.position = worst.position.max(gaps.position);
    }
    println!(
        "指定どおりに折り上がった場合の隔たりの最大: 数{:e} 長さ{:e} 太さ{:e} 位置{:e}",
        worst.count, worst.length, worst.width, worst.position
    );
}

// ---------------------------------------------------------------------------
// 合格条件4: わざと崩すと、対応する物差しが悪化する
// ---------------------------------------------------------------------------

/// 崩し方を1つ受け取り、崩す前後の4つの値を並べて返す。
fn broken(label: &str, break_it: impl Fn(&mut FinishedForm)) -> (FinishGaps, FinishGaps) {
    let skeleton = star_with_positions(4);
    let target = FinishTarget::from_skeleton(&skeleton);
    let base = FinishedForm::matching(&target);
    let mut damaged = base.clone();
    break_it(&mut damaged);
    let before = finish_gaps(&target, &base);
    let after = finish_gaps(&target, &damaged);
    println!(
        "| {label} | {:.6} | {:.6} | {:.6} | {:.6} |",
        after.count, after.length, after.width, after.position
    );
    (before, after)
}

/// 角を1本減らす・長さを半分にする・太さを半分にする・位置をずらす、の4通りで
/// 対応する物差しが悪化することを確かめる。**同時に動く物差しも隠さず出す。**
///
/// 実測(角4本、崩す前は4つとも0.0):
///
/// | 崩し方 | 数 | 長さ | 太さ | 位置 |
/// |---|---:|---:|---:|---:|
/// | 角を1本減らす | 0.250000 | 0.250000 | 0.250000 | 0.250000 |
/// | 長さを半分にする | 0.000000 | 0.125000 | 0.000000 | 0.000000 |
/// | 太さを半分にする | 0.000000 | 0.000000 | 0.125000 | 0.000000 |
/// | 位置を0.1ずらす | 0.000000 | 0.000000 | 0.000000 | 0.008839 |
///
/// 角を1本減らすと4つとも悪化する。先端そのものが無くなると長さも太さも
/// 届かず、位置も測れなくなるためで、物差しの取り違えではない。
#[test]
fn breaking_the_finished_form_worsens_the_matching_gap() {
    println!("\n### わざと崩したときの4指標(角4本)\n");
    println!("| 崩し方 | 数 | 長さ | 太さ | 位置 |");
    println!("|---|---:|---:|---:|---:|");

    // 崩し1: 角を1本減らす → 数が悪化する。
    let (before, after) = broken("角を1本減らす", |f| {
        f.tips.retain(|t| t.leaf_id != 2);
    });
    assert!(
        after.count > before.count,
        "角を減らしたのに数の隔たりが増えなかった: {before:?} → {after:?}"
    );

    // 崩し2: 1本の長さを半分にする → 長さだけが悪化する。
    let (before, after) = broken("長さを半分にする", |f| {
        if let Some(t) = f.tips.iter_mut().find(|t| t.leaf_id == 2) {
            t.length *= 0.5;
        }
    });
    assert!(
        after.length > before.length,
        "長さを半分にしたのに長さの隔たりが増えなかった: {before:?} → {after:?}"
    );
    assert_eq!(after.count, before.count, "長さを変えたのに数が動いた");
    assert_eq!(after.width, before.width, "長さを変えたのに太さが動いた");
    assert_eq!(
        after.position, before.position,
        "長さを変えたのに位置が動いた"
    );

    // 崩し3: 1本の太さを半分にする → 太さだけが悪化する。
    let (before, after) = broken("太さを半分にする", |f| {
        if let Some(t) = f.tips.iter_mut().find(|t| t.leaf_id == 2) {
            t.width *= 0.5;
        }
    });
    assert!(
        after.width > before.width,
        "太さを半分にしたのに太さの隔たりが増えなかった: {before:?} → {after:?}"
    );
    assert_eq!(after.count, before.count, "太さを変えたのに数が動いた");
    assert_eq!(after.length, before.length, "太さを変えたのに長さが動いた");
    assert_eq!(
        after.position, before.position,
        "太さを変えたのに位置が動いた"
    );

    // 崩し4: 1本の位置を0.1ずらす → 位置だけが悪化する(PRO-007: 変化 > 1e-4)。
    let (before, after) = broken("位置を0.1ずらす", |f| {
        if let Some(t) = f.tips.iter_mut().find(|t| t.leaf_id == 2)
            && let Some(p) = t.pos
        {
            t.pos = Some(TipPos2d::new(p.x + 0.1, p.y));
        }
    });
    assert!(
        after.position - before.position > 1e-4,
        "位置を0.1ずらしたのに位置の隔たりが1e-4を超えて変わらなかった: {before:?} → {after:?}"
    );
    assert_eq!(after.count, before.count, "位置を変えたのに数が動いた");
    assert_eq!(after.length, before.length, "位置を変えたのに長さが動いた");
    assert_eq!(after.width, before.width, "位置を変えたのに太さが動いた");
}

/// 紙の上で先端どうしが近すぎると、届く長さが短くなって長さ・太さが悪化する。
///
/// 実測: 角2本(長さ1.0・太さ1.0)、縮尺0.4、必要な距離 `0.4×2.0 = 0.8` に対し
/// 中心の距離を `0.5` にすると、足りない `0.3` を2本で半分ずつ負担して
/// 半径は `0.4 → 0.25`(届いた割合0.625)。長さ・太さの隔たりはどちらも
/// `0.375`、数と位置は動かない。
#[test]
fn limbs_that_sit_too_close_lose_length_and_width() {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for id in 1..=2 {
        nodes.push(SkeletonNode::new(id, Some(0), 1.0));
    }
    let skeleton = Skeleton { nodes };
    let packing = Packing {
        scale: 0.4,
        centers: vec![(1, [0.25, 0.5]), (2, [0.75, 0.5])],
        violation: 0.3,
        circles: Vec::new(),
    };
    let result = generate(&skeleton, &packing, PAPER.0, PAPER.1).expect("生成に失敗した");
    let target = FinishTarget::from_skeleton(&skeleton);
    let form = FinishedForm::from_proposal(&skeleton, &packing, &result);

    for tip in &form.tips {
        assert!(
            tip.is_present(),
            "近すぎるだけで角が消えた: {:?}",
            form.tips
        );
        assert!(
            (tip.length - 0.625).abs() < 1e-12,
            "届いた長さが実測値と違う: {tip:?}"
        );
    }
    assert_eq!(count_gap(&target, &form), 0.0, "角の本数は減っていない");
    assert!(
        (length_gap(&target, &form) - 0.375).abs() < 1e-12,
        "長さの隔たりが実測値と違う: {}",
        length_gap(&target, &form)
    );
    assert!(
        (width_gap(&target, &form) - 0.375).abs() < 1e-12,
        "太さの隔たりが実測値と違う: {}",
        width_gap(&target, &form)
    );
}

// ---------------------------------------------------------------------------
// 合格条件5: 同じ入力を10回測って同じ値
// ---------------------------------------------------------------------------

/// 実測: 葉1・4・12本の3通りそれぞれで10回、4つの値が完全一致(のべ30回)。
#[test]
fn the_same_input_gives_the_same_four_values_ten_times() {
    for n in [1u32, 4, 12] {
        let mut first: Option<FinishGaps> = None;
        for run in 1..=10 {
            let skeleton = star_with_positions(n);
            let (packing, result) = propose(&skeleton);
            let target = FinishTarget::from_skeleton(&skeleton);
            let (center, points) = tip_points_as_specified(&target);
            let form = FinishedForm::from_proposal(&skeleton, &packing, &result)
                .with_tip_points(&target, center, &points);
            let gaps = finish_gaps(&target, &form);
            match first {
                None => first = Some(gaps),
                Some(f) => assert_eq!(gaps, f, "角{n}本: {run}回目の値が1回目と違う"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 合格条件: 先端と紙の場所の対応は作業9の記録をそのまま使う
// ---------------------------------------------------------------------------

/// 測った先端の材料点が、作業9が残した対応(`LeafSite`)と同じ点であること。
/// あわせて、紙の上の配置の中心を完成位置として使う道が無いことを見る(PRO-007)。
#[test]
fn the_measurement_follows_the_recorded_leaf_correspondence() {
    for n in 1..=12u32 {
        let skeleton = star_with_positions(n);
        let (packing, result) = propose(&skeleton);
        let form = FinishedForm::from_proposal(&skeleton, &packing, &result);
        for site in &result.sites {
            let tip = form
                .tip(site.circle.leaf_id)
                .unwrap_or_else(|| panic!("角{n}本: 葉{}の測定が無い", site.circle.leaf_id));
            assert_eq!(
                tip.material_vertex,
                site.vertex.map(|v| v.id),
                "角{n}本: 葉{}の材料点が作業9の記録と違う",
                site.circle.leaf_id
            );
            assert!(
                tip.pos.is_none(),
                "角{n}本: 紙の上の配置から完成位置を作ってしまっている: {tip:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 位置を指定していない先端・測っていない先端の扱い
// ---------------------------------------------------------------------------

/// 位置を指定していない先端は、位置の物差しの母数から外す。
/// 1本も指定が無ければ 0.0 を返し、順位付けに影響しない。
#[test]
fn tips_without_a_specified_position_are_left_out() {
    let mut skeleton = star_with_positions(4);
    // 葉2と葉4の位置指定を外す。
    for node in &mut skeleton.nodes {
        if node.id == 2 || node.id == 4 {
            node.tip_pos_2d = None;
        }
    }
    let target = FinishTarget::from_skeleton(&skeleton);
    assert_eq!(
        target.tips.iter().filter(|t| t.pos.is_some()).count(),
        2,
        "位置を指定した先端が2本になっていない"
    );

    // 指定のある2本だけ合っていれば最良。指定の無い2本は測っていなくてよい。
    let mut form = FinishedForm::matching(&target);
    assert_eq!(
        position_gap(&target, &form),
        0.0,
        "指定の無い先端が効いている"
    );

    // 指定のある1本を、いちばん遠い扱いにする(測っていない)。
    if let Some(t) = form.tips.iter_mut().find(|t| t.leaf_id == 1) {
        t.pos = None;
    }
    assert!(
        (position_gap(&target, &form) - 0.5).abs() < 1e-12,
        "測っていない先端がいちばん遠い(1.0)として数えられていない: {}",
        position_gap(&target, &form)
    );

    // 位置の指定が1本も無ければ 0.0。
    let mut bare = star_with_positions(3);
    for node in &mut bare.nodes {
        node.tip_pos_2d = None;
    }
    let bare_target = FinishTarget::from_skeleton(&bare);
    let bare_form = FinishedForm::matching(&bare_target);
    assert_eq!(position_gap(&bare_target, &bare_form), 0.0);
}

/// 位置の隔たりは、枠の中で測れているかぎり 0.0〜1.0 に収まる。
/// いちばん遠い組(枠の対角)でちょうど 1.0 になる。
#[test]
fn the_position_gap_is_one_at_the_far_corner_of_the_frame() {
    let target = FinishTarget {
        tips: vec![ori3_propose::TargetTip {
            leaf_id: 1,
            length: 1.0,
            width: 1.0,
            pos: Some(TipPos2d::new(-1.0, -1.0)),
        }],
    };
    let form = FinishedForm {
        tips: vec![MeasuredTip {
            leaf_id: 1,
            material_vertex: None,
            length: 1.0,
            width: 1.0,
            pos: Some(TipPos2d::new(1.0, 1.0)),
        }],
    };
    assert!((position_gap(&target, &form) - 1.0).abs() < 1e-12);
    assert!((POSITION_GAP_MAX - 2.828_427_124_746_190_3).abs() < 1e-12);
}

/// 折り上げた点を入れるとき、単位や置き場所が変わっても同じ位置になること。
/// 胴の中心を原点に置き、いちばん遠い先端を指定のいちばん遠い先端にそろえる。
#[test]
fn measured_tip_points_are_placed_in_the_same_frame_as_the_specification() {
    let skeleton = star_with_positions(4);
    let target = FinishTarget::from_skeleton(&skeleton);
    let base = FinishedForm::matching(&target);

    let small =
        base.clone()
            .with_tip_points(&target, [0.0, 0.0], &[(1, [0.9, 0.0]), (2, [0.0, 0.9])]);
    let large = base.clone().with_tip_points(
        &target,
        [100.0, -50.0],
        &[(1, [190.0, -50.0]), (2, [100.0, 40.0])],
    );
    for id in [1, 2] {
        let a = small
            .tip(id)
            .and_then(|t| t.pos)
            .expect("位置が入っていない");
        let b = large
            .tip(id)
            .and_then(|t| t.pos)
            .expect("位置が入っていない");
        assert!(
            (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12,
            "大きさや置き場所で位置が変わった: {a:?} / {b:?}"
        );
    }

    // 知らない葉IDの点は捨てる。数にならない点は「測っていない」のまま。
    let odd = base.clone().with_tip_points(
        &target,
        [0.0, 0.0],
        &[(999, [1.0, 1.0]), (3, [f64::NAN, 0.0])],
    );
    assert!(odd.tip(999).is_none(), "知らない葉IDが増えた");
    assert_eq!(
        odd.tip(3).and_then(|t| t.pos),
        base.tip(3).and_then(|t| t.pos),
        "数にならない点で位置が書き換わった"
    );
}
