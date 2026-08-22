//! 折り方の計画に使う追跡情報(作業17)のテスト。

use std::collections::BTreeSet;

use ori3_model::EdgeKind;
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::{
    CreaseRole, FinishedPart, MoleculeCorner, MoleculeRelation, ProposalResult, check_trace,
    generate, pack,
};

mod support;

/// 根に葉を`n`本ぶら下げた星形の骨格(作業9の記録と同じ形)。
fn star(n: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), 1.0));
    }
    Skeleton { nodes }
}

/// 作業9の記録と同じ条件(紙1.0×1.0、seed 2026、やり直し8回、先頭候補)。
fn build(n: u32) -> ProposalResult {
    let s = star(n);
    let ps = pack(&s, 1.0, 1.0, 2026, 8);
    assert!(!ps.is_empty(), "葉{n}本の配置に失敗した");
    generate(&s, &ps[0], 1.0, 1.0).unwrap_or_else(|e| panic!("葉{n}本の生成に失敗した: {e}"))
}

/// 紙の四隅に先端を1本ずつ置いた形(ロードマップの「四隅4葉」)。
fn four_corners() -> ProposalResult {
    let s = star(4);
    let packing = ori3_propose::Packing {
        scale: 0.5,
        centers: vec![
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [1.0, 1.0]),
            (4, [0.0, 1.0]),
        ],
        violation: 0.0,
        circles: Vec::new(),
    };
    generate(&s, &packing, 1.0, 1.0).expect("四隅4葉の生成に失敗した")
}

// ---------------------------------------------------------------------------
// 合格条件1: 葉1〜12本の12通りすべてで、追跡情報が欠けなく付く
// ---------------------------------------------------------------------------

/// 分子1つにつき、角3つ・領域4つ・折り線7本(紙の縁と重なる軸線を引かない分だけ減る)
/// が欠けずに付いていること。欠損・重複ともに0件。
#[test]
fn every_molecule_carries_complete_tracking_for_one_to_twelve_leaves() {
    let mut shapes = 0usize;
    let mut molecules = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        assert!(
            !r.trace.molecules.is_empty(),
            "葉{n}本で分子が1つも記録されていない"
        );
        for (i, m) in r.trace.molecules.iter().enumerate() {
            assert_eq!(m.index, i, "葉{n}本: 分子の番号が並び順と違う");
            assert_eq!(m.regions.len(), 4, "葉{n}本: 分子{i}の領域が4つでない");
            assert!(m.hinge_side < 3, "葉{n}本: 分子{i}のちょうつがい辺が範囲外");

            // 領域: 番号が0..4で重複なし、完成部位と表裏が入っている。
            let indices: BTreeSet<usize> = m.regions.iter().map(|r| r.index).collect();
            assert_eq!(indices, (0..4).collect::<BTreeSet<usize>>());
            for region in &m.regions {
                assert_eq!(region.polygon.len(), 3, "領域が三角形でない");
                assert!(region.axial_side < 3, "領域の外周の辺が範囲外");
                assert!(
                    region.axial_span[0] <= region.axial_span[1],
                    "領域の区間が逆向き"
                );
            }

            // 折り線: 軸線・稜線・ちょうつがい線の本数が形どおり。
            let count = |role: CreaseRole| m.creases.iter().filter(|c| c.role == role).count();
            assert_eq!(count(CreaseRole::Ridge), 3, "稜線が3本でない");
            assert_eq!(count(CreaseRole::Hinge), 1, "ちょうつがい線が1本でない");
            assert!(count(CreaseRole::Axial) <= 3, "軸線が3本を超えた");
            for c in &m.creases {
                assert_eq!(c.molecule, i, "折り線が別の分子の番号を持っている");
                assert!(!c.regions.is_empty(), "折り線に接する領域が0件");
                assert_eq!(c.regions.len(), c.sides.len(), "領域と表裏の件数が違う");
                let want = match c.kind {
                    EdgeKind::Mountain => 180.0,
                    EdgeKind::Valley => -180.0,
                    other => panic!("折り線の山谷が想定外: {other:?}"),
                };
                assert_eq!(c.target_angle_deg, want, "目標角が山谷と合っていない");
                let mut sorted = c.regions.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted, c.regions, "接する領域に重複がある");
            }
            molecules += 1;
        }
        shapes += 1;
    }
    assert_eq!(shapes, 12, "12通りすべてを見ていない");
    println!("葉1〜12本 合計 分子{molecules}件");
}

/// 各先端の材料点を囲む分子(作業9の `LeafSite::molecules`)が、
/// 追跡情報の同じ番号の分子でも、その先端を角に持っていること。
/// 作業9の対応と作業17の追跡情報が同じものを指しているかの点検。
#[test]
fn leaf_sites_and_molecule_tracking_point_at_the_same_molecules() {
    let mut checked = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        for site in &r.sites {
            for &mi in &site.molecules {
                let m = r
                    .trace
                    .molecule(mi)
                    .unwrap_or_else(|| panic!("葉{n}本: 分子{mi}の追跡情報が無い"));
                assert!(
                    m.corners
                        .iter()
                        .any(|c| c.leaf_id() == Some(site.circle.leaf_id)),
                    "葉{n}本: 分子{mi}が先端{}を角に持っていない",
                    site.circle.leaf_id
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 78, "見た組が{checked}件しかない");
    println!("先端と分子の組 {checked}件を照合");
}

/// 骨格の先端どうしを結ぶ軸線には、必ず由来する先端と骨格の辺が入っていること。
/// 紙の隅から出る折り線は、骨格に対応がないので余りの紙として区別されること。
#[test]
fn creases_carry_the_skeleton_they_come_from() {
    let mut with_leaves = 0usize;
    let mut margin = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        for m in &r.trace.molecules {
            for c in &m.creases {
                match c.role {
                    CreaseRole::Ridge => {
                        // 稜線は角1つに由来する。その角が先端なら1本、紙の隅なら0本。
                        assert!(c.leaves.len() <= 1, "稜線が2本以上の先端に由来している");
                    }
                    CreaseRole::Axial | CreaseRole::Hinge => {
                        assert!(c.leaves.len() <= 2, "軸線・ちょうつがい線の先端が3本以上");
                    }
                }
                if c.leaves.is_empty() {
                    margin += 1;
                } else {
                    with_leaves += 1;
                }
                // 由来する骨格の辺は、由来する先端から見て正しいこと。
                for e in &c.body_edges {
                    assert!(
                        star(n).node(*e).is_some(),
                        "葉{n}本: 骨格に無い辺{e}を由来として記録した"
                    );
                }
            }
            // 領域の完成部位も同じく骨格へたどれること。
            for region in &m.regions {
                match &region.part {
                    FinishedPart::Path {
                        from_leaf,
                        to_leaf,
                        body_edges,
                    } => {
                        assert!(from_leaf < to_leaf, "経路の両端が正規化されていない");
                        assert!(!body_edges.is_empty(), "経路の胴が空");
                    }
                    FinishedPart::Margin { corners } => {
                        assert!(
                            corners.iter().any(|c| c.leaf_id().is_none()),
                            "両端が先端なのに余りの紙になっている"
                        );
                    }
                }
            }
        }
    }
    println!("骨格へたどれる折り線 {with_leaves}本 / 余りの紙の折り線 {margin}本");
    assert!(with_leaves > 0, "骨格へたどれる折り線が0本");
}

/// 隣り合う分子が、完成形で重なるのか並ぶのかを必ず持っていること。
/// 辺を共有する組は、その軸線で紙が折り返るので必ず重なる。
#[test]
fn neighbouring_molecules_say_whether_they_stack_or_sit_side_by_side() {
    let mut stacked = 0usize;
    let mut side_by_side = 0usize;
    let mut edge_pairs = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        for pair in &r.trace.neighbors {
            assert!(pair.a < pair.b, "組の並びが昇順でない");
            assert!(!pair.shared_corners.is_empty(), "共有する角が0件");
            if pair.shared_edge.is_some() {
                edge_pairs += 1;
                assert_eq!(
                    pair.relation,
                    MoleculeRelation::Stacked,
                    "葉{n}本: 辺を共有する分子{}と{}が重ならない扱いになっている",
                    pair.a,
                    pair.b
                );
            }
            match pair.relation {
                MoleculeRelation::Stacked => stacked += 1,
                MoleculeRelation::SideBySide => side_by_side += 1,
            }
        }
    }
    println!("重なる組 {stacked}件 / 並ぶ組 {side_by_side}件 / うち辺を共有 {edge_pairs}件");
    assert!(edge_pairs > 0, "辺を共有する組が0件");
}

/// 枝分かれのある骨格でも、重なる・並ぶの判定が骨格のつながりどおりになること。
///
/// 胴を共有しない先端どうし(ここでは11・12の組と20・21の組)は、完成形では
/// 軸に沿って並ぶだけで重ならない。角も辺も共有しない分子はそもそも組にならず、
/// 角を共有する組は共有した先端の分だけ必ず重なる。
#[test]
fn branching_skeleton_separates_stacking_from_sitting_side_by_side() {
    let s = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(10, Some(0), 0.6),
            SkeletonNode::new(11, Some(10), 0.8),
            SkeletonNode::new(12, Some(10), 0.8),
            SkeletonNode::new(20, Some(0), 0.8),
            SkeletonNode::new(21, Some(0), 0.8),
        ],
    };
    assert_eq!(s.path_edges(11, 12), vec![11, 12]);
    assert_eq!(s.path_edges(20, 21), vec![20, 21]);

    let ps = pack(&s, 1.0, 1.0, 2026, 8);
    assert!(!ps.is_empty(), "枝分かれの形の配置に失敗した");
    let r = generate(&s, &ps[0], 1.0, 1.0).expect("枝分かれの形の生成に失敗した");
    let mut stacked = 0usize;
    let mut side_by_side = 0usize;
    for pair in &r.trace.neighbors {
        // 重なると言うからには、辺を共有しているか胴を共有しているかのどちらか。
        match pair.relation {
            MoleculeRelation::Stacked => {
                assert!(
                    pair.shared_edge.is_some() || !pair.shared_body_edges.is_empty(),
                    "重なる根拠が無い組: {}と{}",
                    pair.a,
                    pair.b
                );
                stacked += 1;
            }
            MoleculeRelation::SideBySide => {
                assert!(pair.shared_edge.is_none(), "辺を共有しているのに並ぶ扱い");
                assert!(
                    pair.shared_body_edges.is_empty(),
                    "胴を共有しているのに並ぶ扱い"
                );
                side_by_side += 1;
            }
        }
    }
    println!(
        "枝分かれの形: 分子{}件、重なる組{stacked}件、並ぶ組{side_by_side}件",
        r.trace.molecules.len()
    );
    assert!(stacked > 0, "重なる組が0件");
}

// ---------------------------------------------------------------------------
// 合格条件2: 今までの展開図が変わらない
// ---------------------------------------------------------------------------

/// 追跡情報を足す前に取った記録(作業9の `cp-baseline-1-12.json`)と、
/// 葉1〜12本の12通りの展開図が変わっていないこと。
///
/// **頂点と辺の個数・番号・並び・つながり・山谷の種類は完全一致**を求める。
/// **座標だけ** [`support::CP_POS_TOL`] の許容差で比べる
/// (理由と実測、わざと壊して落ちることを確かめた記録は同定数のコメント)。
#[test]
fn crease_patterns_stay_identical_to_the_recorded_baseline() {
    let baseline = support::read_baseline();
    assert_eq!(baseline.len(), 12, "記録が12件ない");
    let mut same = 0usize;
    let mut seen_vertices = 0usize;
    let mut seen_edges = 0usize;
    let mut worst = 0.0f64;
    for entry in &baseline {
        let (vertices, edges, gap) =
            support::assert_cp_matches_baseline(entry, &build(entry.leaves).cp);
        seen_vertices += vertices;
        seen_edges += edges;
        worst = worst.max(gap);
        same += 1;
    }
    assert_eq!(same, 12, "一致した件数が12件でない");
    // 空回りしていないことの見張り。実測で のべ頂点240個・のべ辺532本。
    // どちらも記録の中身そのもので、計算機が変わっても変わらない。
    assert_eq!(seen_vertices, 240, "見た頂点がのべ240個でない");
    assert_eq!(seen_edges, 532, "見た辺がのべ532本でない");
    println!(
        "記録との突き合わせ: 12通り、のべ頂点{seen_vertices}個・のべ辺{seen_edges}本。\
         座標の差の最大 = {worst:.3e}(許容 {:.0e})",
        support::CP_POS_TOL
    );
}

// ---------------------------------------------------------------------------
// 合格条件3: 同じ入力を10回生成して、追跡情報が毎回同じ
// ---------------------------------------------------------------------------

#[test]
fn the_same_input_gives_the_same_tracking_ten_times() {
    for n in [1u32, 4, 6, 12] {
        let first = serde_json::to_string(&build(n).trace).expect("追跡情報を書き出せない");
        let mut same = 0usize;
        for round in 1..=10 {
            let again = serde_json::to_string(&build(n).trace).expect("追跡情報を書き出せない");
            assert_eq!(again, first, "葉{n}本の{round}回目で追跡情報が変わった");
            same += 1;
        }
        assert_eq!(same, 10, "葉{n}本で10回そろっていない");
    }
}

// ---------------------------------------------------------------------------
// 合格条件4: 山谷の付き方が追跡情報と矛盾しないこと
// ---------------------------------------------------------------------------

/// 展開図の折り線が、すべてどれか1つの分子の追跡情報に載っていること(未割当0)。
/// 山谷も追跡情報と一致していること。
#[test]
fn every_crease_in_the_pattern_is_tracked_with_the_same_mountain_valley() {
    let mut total = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        let checks = check_trace(&r.cp, &r.trace);
        assert_eq!(
            checks.unassigned_creases, 0,
            "葉{n}本: 追跡情報に載っていない折り線が{}本",
            checks.unassigned_creases
        );
        assert_eq!(
            checks.kind_mismatches, 0,
            "葉{n}本: 追跡情報と展開図で山谷が違う折り線が{}本",
            checks.kind_mismatches
        );
        assert_eq!(checks.missing_fields, 0, "葉{n}本: 欄が欠けた記録がある");
        total += checks.creases;
    }
    println!("葉1〜12本 合計 折り線{total}本、未割当0本、山谷の食い違い0本");
}

/// 分子の内心のまわりが「山3+谷1」になっていること(前川定理)。
/// 同じ軸線を共有する分子どうしで、山谷と完成部位の記録がそろっていること。
#[test]
fn mountain_valley_is_consistent_with_the_tracking_inside_and_between_molecules() {
    for n in 1..=12u32 {
        let r = build(n);
        let checks = check_trace(&r.cp, &r.trace);
        assert_eq!(
            checks.maekawa_violations, 0,
            "葉{n}本: 内心のまわりが山3+谷1になっていない分子が{}件",
            checks.maekawa_violations
        );
        assert_eq!(
            checks.shared_kind_conflicts, 0,
            "葉{n}本: 同じ軸線を違う山谷で記録した組が{}件",
            checks.shared_kind_conflicts
        );
        assert_eq!(
            checks.shared_part_conflicts, 0,
            "葉{n}本: 同じ軸線の両側で完成部位が食い違った組が{}件",
            checks.shared_part_conflicts
        );
    }
}

/// 四隅4葉(ロードマップの受け入れ条件)では平坦に折りにくい点が0か所で、
/// 表裏の塗り分けも食い違わないこと。
#[test]
fn four_corner_leaves_have_no_flat_fold_violation_and_no_side_conflict() {
    let r = four_corners();
    assert_eq!(r.violations, 0, "四隅4葉で平坦に折りにくい点がある");
    let checks = check_trace(&r.cp, &r.trace);
    assert_eq!(
        checks.unassigned_creases, 0,
        "四隅4葉で未割当の折り線がある"
    );
    assert_eq!(
        checks.side_conflicts, 0,
        "四隅4葉で表裏の塗り分けが食い違った"
    );
    println!(
        "四隅4葉: 分子{}件、折り線{}本",
        r.trace.molecules.len(),
        checks.creases
    );
}

/// 表裏の塗り分けが食い違う件数を、平坦に折りにくい点の数と並べて記録する。
///
/// 折り線をまたぐと紙は必ず裏返るので、食い違いが出る展開図は平坦に折り切れない。
/// **この食い違いは追跡情報の欠陥ではなく、いまの山谷の付け方が完成形を見ずに
/// 「軸線=谷・稜線=山・ちょうつがい=谷」と決め打ちしていることの現れである。**
/// 作業17では直さない(直すのは折り順を決める作業18以降)。
///
/// 実測(手元、`cargo test -p ori3-propose --test fold_plan_trace -- --nocapture`):
/// 下の表のとおり。食い違いが出るのは平坦に折りにくい点があるときだけで、
/// 折りにくい点が0か所の葉1・2・4本では食い違いも0件だった。
#[test]
fn side_conflicts_are_recorded_next_to_the_flat_fold_violations() {
    let mut rows = Vec::new();
    for n in 1..=12u32 {
        let r = build(n);
        rows.push((n, r.violations, r.trace.side_conflicts));
        println!(
            "葉{n}本: 平らに折りにくい点{}か所、表裏の食い違い{}件、分子{}件",
            r.violations,
            r.trace.side_conflicts,
            r.trace.molecules.len()
        );
    }
    for (n, violations, conflicts) in rows {
        if violations == 0 {
            assert_eq!(
                conflicts, 0,
                "葉{n}本: 平らに折りにくい点が0か所なのに表裏が食い違った"
            );
        }
    }
}

/// 先端の材料点のまわりの山谷を、分子をまたいで数えて記録する。
///
/// 材料点に集まるのは「そこから出る軸線(いまは必ず谷)」と「そこから内心へ
/// 向かう稜線(いまは必ず山)」だけで、紙の内側にある先端では本数が必ず同じに
/// なる。したがって前川定理(|山−谷|=2)は**構造的に満たしようがない**。
/// これは追跡情報の欠陥ではなく、**山谷を完成形・重なり順・折る向きから決めず、
/// 「軸線=谷・稜線=山」と決め打ちしていることの直接の帰結**である。
/// 作業17では直さない(直すのは折り順を決める作業18以降)。
///
/// 実測(手元、`cargo test -p ori3-propose --test fold_plan_trace -- --nocapture`):
/// 下の表のとおり、紙の内側にある先端は**全件**が前川定理を満たさなかった。
#[test]
fn limb_tips_inside_the_paper_cannot_satisfy_maekawa_with_the_current_rule() {
    let mut interior_total = 0usize;
    let mut violation_total = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        let checks = check_trace(&r.cp, &r.trace);
        println!(
            "葉{n}本: 紙の内側の先端{}本、うち前川定理を満たさない{}本",
            checks.interior_limbs, checks.limb_maekawa_violations
        );
        interior_total += checks.interior_limbs;
        violation_total += checks.limb_maekawa_violations;
    }
    println!("葉1〜12本 合計: 紙の内側の先端{interior_total}本、満たさない{violation_total}本");
    assert!(interior_total > 0, "紙の内側にある先端が1本も無い");
    assert_eq!(
        violation_total, interior_total,
        "紙の内側の先端の一部だけが前川定理を満たさない状態になった。\
         山谷の付け方が変わったなら、この記録も測り直して更新すること"
    );
}

// ---------------------------------------------------------------------------
// 追跡情報そのものの点検
// ---------------------------------------------------------------------------

/// 分子の角がすべて「先端」か「紙の隅」のどちらかにたどれること(不明0件)。
#[test]
fn every_molecule_corner_is_either_a_limb_or_a_paper_corner() {
    let mut unknown = 0usize;
    let mut leaves = 0usize;
    let mut corners = 0usize;
    for n in 1..=12u32 {
        let r = build(n);
        for m in &r.trace.molecules {
            for c in m.corners {
                match c {
                    MoleculeCorner::Leaf { .. } => leaves += 1,
                    MoleculeCorner::PaperCorner { .. } => corners += 1,
                    MoleculeCorner::Unknown => unknown += 1,
                }
            }
        }
    }
    assert_eq!(unknown, 0, "由来をたどれない角が{unknown}件");
    println!("角: 先端{leaves}件 / 紙の隅{corners}件 / 不明{unknown}件");
}

/// 骨格の経路が、木のつながりどおりに求まっていること。
#[test]
fn skeleton_path_edges_follow_the_tree() {
    // 根0 - 中間10 - 葉11 / 葉12、根0 - 葉20
    let s = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(10, Some(0), 1.0),
            SkeletonNode::new(11, Some(10), 1.0),
            SkeletonNode::new(12, Some(10), 1.0),
            SkeletonNode::new(20, Some(0), 1.0),
        ],
    };
    assert_eq!(
        s.path_edges(11, 12),
        vec![11, 12],
        "共通の親までで止まらない"
    );
    assert_eq!(
        s.path_edges(11, 20),
        vec![10, 11, 20],
        "根をまたぐ経路が違う"
    );
    assert_eq!(s.path_edges(11, 11), Vec::<u32>::new(), "同じ葉が空でない");
    assert_eq!(
        s.path_edges(11, 999),
        Vec::<u32>::new(),
        "無い節点が空でない"
    );
}
