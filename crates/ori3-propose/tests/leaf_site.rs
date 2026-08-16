//! 先端(葉)と展開図の材料点の対応(作業9 / PRO-007)のテスト。

use std::collections::BTreeSet;

use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::{generate, pack};

mod support;

use support::{CpBaseline, baseline_path, read_baseline};

/// 根に葉を`n`本ぶら下げた星形の骨格。
fn star(n: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), 1.0));
    }
    Skeleton { nodes }
}

/// 記録と同じ条件(紙1.0×1.0、seed 2026、やり直し8回、先頭候補)で1件作る。
fn build(n: u32) -> (ori3_propose::Packing, ori3_propose::ProposalResult) {
    let s = star(n);
    let ps = pack(&s, 1.0, 1.0, 2026, 8);
    assert!(!ps.is_empty(), "葉{n}本の配置に失敗した");
    let r = generate(&s, &ps[0], 1.0, 1.0).unwrap_or_else(|e| panic!("葉{n}本の生成に失敗した: {e}"));
    (ps.into_iter().next().unwrap(), r)
}

/// 記録の作り直し。展開図をわざと変えたいときだけ、明示的に実行する。
/// `cargo test -p ori3-propose --test leaf_site -- --ignored regenerate_cp_baseline`
#[test]
#[ignore = "記録を書き換えるため、明示的に実行したときだけ動かす"]
fn regenerate_cp_baseline() {
    let entries: Vec<CpBaseline> = (1..=12u32)
        .map(|n| CpBaseline {
            leaves: n,
            cp_json: serde_json::to_string(&build(n).1.cp).expect("展開図を書き出せない"),
        })
        .collect();
    let text = serde_json::to_string_pretty(&entries).expect("記録を書き出せない");
    std::fs::write(baseline_path(), text).expect("記録を保存できない");
}

/// 合格条件5: 対応を足す前と後で、葉1〜12本の12通りの展開図が変わらない。
/// 記録は対応を足す前の実装で作ったもの。
///
/// **頂点と辺の個数・番号・並び・つながり・山谷の種類は完全一致**を求める。
/// **座標だけ** [`support::CP_POS_TOL`] の許容差で比べる
/// (理由と実測は同定数のコメント。以前はここも書き出した文字列の完全一致で
/// 比べており、最下位1桁の違いでCIが落ちた)。
#[test]
fn crease_patterns_for_one_to_twelve_leaves_are_unchanged() {
    let baseline = read_baseline();
    assert_eq!(baseline.len(), 12, "記録が12件ない");
    let mut same = 0usize;
    let mut seen_vertices = 0usize;
    let mut seen_edges = 0usize;
    let mut worst = 0.0f64;
    for entry in &baseline {
        let (_, r) = build(entry.leaves);
        let (vertices, edges, gap) = support::assert_cp_matches_baseline(entry, &r.cp);
        seen_vertices += vertices;
        seen_edges += edges;
        worst = worst.max(gap);
        same += 1;
    }
    assert_eq!(same, 12, "展開図が一致した件数が12件でない");
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

/// 記録が葉1〜12本を1本ずつ、重複なく覆っていること(記録そのものの点検)。
#[test]
fn baseline_covers_one_to_twelve_leaves_exactly_once() {
    let baseline = read_baseline();
    let leaves: BTreeSet<u32> = baseline.iter().map(|e| e.leaves).collect();
    assert_eq!(leaves, (1..=12).collect::<BTreeSet<u32>>());
    assert_eq!(leaves.len(), baseline.len(), "記録に重複がある");
}

/// 合格条件1: 葉1〜12本の12通りすべてで、各葉IDが展開図の材料点へちょうど1回
/// 対応する。欠け0件・重複0件。
#[test]
fn every_leaf_maps_to_exactly_one_material_point_for_one_to_twelve_leaves() {
    let mut covered = 0usize;
    for n in 1..=12u32 {
        let (_, r) = build(n);
        let expected: BTreeSet<u32> = star(n).leaves().into_iter().collect();
        assert_eq!(expected.len(), n as usize, "葉{n}本の骨格の葉が{n}本でない");

        assert_eq!(r.sites.len(), n as usize, "葉{n}本で対応が{}件", r.sites.len());
        let got: BTreeSet<u32> = r.sites.iter().map(|s| s.circle.leaf_id).collect();
        assert_eq!(got, expected, "葉{n}本で対応する葉IDの顔ぶれが違う");
        assert_eq!(got.len(), r.sites.len(), "葉{n}本で同じ葉IDの対応が重複した");

        for site in &r.sites {
            let v = site.vertex.unwrap_or_else(|| {
                panic!("葉{n}本: 先端{}の材料点が無い(欠損)", site.circle.leaf_id)
            });
            assert!(
                r.cp.vertices.iter().any(|x| x.id == v.id),
                "葉{n}本: 先端{}の材料点{}が展開図に無い",
                site.circle.leaf_id,
                v.id
            );
        }
        covered += 1;
    }
    assert_eq!(covered, 12, "12通りすべてを見ていない");
}

/// 合格条件2: 対応先の点と、配置で得た円の中心の距離が 1e-7 未満。
///
/// 実測(手元、`cargo test -p ori3-propose --test leaf_site -- --nocapture`):
/// 葉1〜12本の12通り・のべ78本の先端で、ずれの最大は **5.551115123125783e-17**
/// (葉6本のとき)。他の11通りはすべて **0.0**(完全一致)だった。
/// 上限1e-7は `triangulate::MERGE_TOL` と同じ値で、同じ点とみなす距離にあたる。
/// 実測の最大値は上限の約1/1,800,000,000で、余裕は9桁ある。
#[test]
fn material_point_sits_on_the_packed_circle_center() {
    const LIMIT: f64 = 1e-7;
    let mut worst = 0.0f64;
    let mut checked = 0usize;
    for n in 1..=12u32 {
        let (packing, r) = build(n);
        let mut worst_here = 0.0f64;
        for site in &r.sites {
            let v = site.vertex.expect("材料点が無い");
            let center = packing.centers[site.circle.circle_index].1;
            // 記録したずれと、配置の中心から測り直したずれの両方を見る。
            let measured = (v.pos[0] - center[0]).hypot(v.pos[1] - center[1]);
            assert!(
                v.gap < LIMIT && measured < LIMIT,
                "葉{n}本: 先端{}のずれが大きい(記録{}, 測り直し{measured})",
                site.circle.leaf_id,
                v.gap
            );
            assert_eq!(v.gap, measured, "記録したずれと測り直した値が違う");
            worst_here = worst_here.max(measured);
            checked += 1;
        }
        println!("葉{n}本: 先端{}本、最大のずれ {worst_here:e}", r.sites.len());
        worst = worst.max(worst_here);
    }
    println!("12通り合計 先端{checked}本、全体の最大のずれ {worst:e}");
    assert_eq!(checked, 78, "1+2+...+12=78本を見ていない");
    assert!(worst < LIMIT, "全体の最大のずれ{worst}が{LIMIT}以上");
}

/// 合格条件3: 同じ入力を10回生成して、対応が毎回同じ。
#[test]
fn the_same_input_gives_the_same_correspondence_ten_times() {
    for n in [1u32, 6, 12] {
        let first = serde_json::to_string(&build(n).1.sites).expect("対応を書き出せない");
        let mut same = 0usize;
        for round in 1..=10 {
            let again = serde_json::to_string(&build(n).1.sites).expect("対応を書き出せない");
            assert_eq!(again, first, "葉{n}本の{round}回目で対応が変わった");
            same += 1;
        }
        assert_eq!(same, 10, "葉{n}本で10回そろっていない");
    }
}

/// どの先端も、少なくとも1つの分子(軸多角形の三角形)に囲まれていること。
/// 分子が0個なら、その先端の材料は折り線で切り出されていないことになる。
#[test]
fn every_leaf_belongs_to_at_least_one_molecule() {
    for n in 1..=12u32 {
        let (_, r) = build(n);
        for site in &r.sites {
            assert!(
                !site.molecules.is_empty(),
                "葉{n}本: 先端{}を囲む分子が0個",
                site.circle.leaf_id
            );
            let mut sorted = site.molecules.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted, site.molecules, "分子の番号が昇順・重複なしでない");
        }
    }
}

/// 配置が返す2つの並び(`centers` と `circles`)が同じ円を指していること。
/// 番号で名指しできるようにしたのが `circles` で、中身は同じでなければならない。
#[test]
fn packed_circles_and_centers_describe_the_same_circles() {
    for n in 1..=12u32 {
        let s = star(n);
        for p in pack(&s, 1.0, 1.0, 2026, 8) {
            assert_eq!(p.circles.len(), p.centers.len(), "葉{n}本で件数が違う");
            for (i, circle) in p.circles.iter().enumerate() {
                assert_eq!(circle.circle_index, i, "円の番号が並び順と違う");
                assert_eq!(circle.leaf_id, p.centers[i].0, "先端IDが違う");
                assert_eq!(circle.center, p.centers[i].1, "中心が違う");
                assert_eq!(
                    circle.radius,
                    p.scale * s.leaf_radius(circle.leaf_id),
                    "半径が縮尺×骨格の半径になっていない"
                );
            }
        }
    }
}

/// 円の記録を持たない`Packing`(手で組み立てたもの・この欄が無かったころの保存)
/// でも、対応が欠けないこと。中心と骨格から同じ内容を組み立て直す。
#[test]
fn packing_without_circle_records_still_yields_the_correspondence() {
    let s = star(4);
    let packed = pack(&s, 1.0, 1.0, 2026, 8).remove(0);
    let bare = ori3_propose::Packing {
        scale: packed.scale,
        centers: packed.centers.clone(),
        violation: packed.violation,
        circles: Vec::new(),
    };
    let with_records = generate(&s, &packed, 1.0, 1.0).unwrap();
    let without_records = generate(&s, &bare, 1.0, 1.0).unwrap();
    assert_eq!(
        without_records.sites, with_records.sites,
        "円の記録の有無で対応が変わった"
    );
    assert_eq!(without_records.sites.len(), 4, "対応が4件でない");
    assert!(
        without_records.sites.iter().all(|s| s.vertex.is_some()),
        "円の記録が無いときに材料点が欠けた"
    );
}
