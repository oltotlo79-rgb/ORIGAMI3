//! 円・川充填(PRO-002 / 要件§8-2)のテスト。

use ori3_propose::packing::{MAX_CANDIDATES, PACK_TOL, Packing, pack};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};

/// 根に葉を`n`本だけぶら下げた星形の骨格(各辺の長さは`len`)。
fn star(n: u32, len: f64) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), len));
    }
    Skeleton { nodes }
}

/// 結果が本当に制約を満たしているかを、骨格から独立に検算する。
fn max_violation(s: &Skeleton, p: &Packing, w: f64, h: f64) -> f64 {
    let mut v: f64 = 0.0;
    for (ia, &(id_a, a)) in p.centers.iter().enumerate() {
        v = v.max(-a[0]).max(a[0] - w).max(-a[1]).max(a[1] - h);
        for &(id_b, b) in &p.centers[ia + 1..] {
            let need = p.scale * s.leaf_distance(id_a, id_b);
            v = v.max(need - (a[0] - b[0]).hypot(a[1] - b[1]));
        }
    }
    v.max(0.0)
}

#[test]
fn two_leaves_on_unit_square_reach_half_scale() {
    let s = star(2, 1.0);
    let out = pack(&s, 1.0, 1.0, 20260806, 8);
    assert!(!out.is_empty());
    // 必要距離は 1+1=2、正方形の対角は√2 なので理論最大は約0.707。
    assert!(out[0].scale >= 0.5, "縮尺が足りない: {:?}", out[0]);
    assert!(out[0].violation <= PACK_TOL, "違反: {}", out[0].violation);
}

#[test]
fn five_leaves_satisfy_all_constraints() {
    let s = star(5, 1.0);
    let out = pack(&s, 1.0, 1.0, 7, 8);
    assert!(!out.is_empty());
    for p in &out {
        assert!(p.scale > 0.0, "縮尺が0: {p:?}");
        assert_eq!(p.centers.len(), 5);
        assert!(p.violation <= PACK_TOL, "報告違反: {}", p.violation);
        let v = max_violation(&s, p, 1.0, 1.0);
        assert!(v <= PACK_TOL, "検算した違反: {v}");
    }
}

#[test]
fn same_seed_gives_same_result() {
    let s = star(6, 0.8);
    let a = pack(&s, 1.0, 0.75, 42, 8);
    let b = pack(&s, 1.0, 0.75, 42, 8);
    assert_eq!(a, b);
    let c = pack(&s, 1.0, 0.75, 43, 8);
    assert_ne!(a, c, "シードを変えても同じ結果になっている");
}

#[test]
fn returns_at_most_four_candidates_in_score_order() {
    let out = pack(&star(4, 1.0), 1.0, 1.0, 1, 8);
    assert!(out.len() <= MAX_CANDIDATES && !out.is_empty());
    for w in out.windows(2) {
        assert!(w[0].scale >= w[1].scale, "縮尺の降順になっていない");
    }
}

#[test]
fn one_to_twelve_leaves_never_panic() {
    for n in 1..=12u32 {
        let s = star(n, 1.0);
        assert_eq!(s.validate(), Ok(()));
        let out = pack(&s, 1.0, 1.0, u64::from(n), 8);
        assert!(!out.is_empty(), "葉{n}本で候補が出ない");
        for p in &out {
            assert!(p.scale.is_finite() && p.scale > 0.0, "葉{n}本: {p:?}");
            assert!(max_violation(&s, p, 1.0, 1.0) <= PACK_TOL, "葉{n}本");
        }
    }
}

#[test]
fn invalid_input_returns_no_candidate() {
    assert!(pack(&Skeleton::default(), 1.0, 1.0, 0, 8).is_empty());
    assert!(pack(&star(2, 1.0), 0.0, 1.0, 0, 8).is_empty());
    // スタート数0でも1回は走らせる。
    assert!(!pack(&star(2, 1.0), 1.0, 1.0, 0, 0).is_empty());
}

/// 頭1・尾1・足4(要件§8の精度目標)。鶴系の基本形に見合う縮尺が出ること。
#[test]
fn bird_base_skeleton_packs_reasonably() {
    let mut nodes = vec![
        SkeletonNode::new(0, None, 0.0),
        SkeletonNode::new(1, Some(0), 0.3),
        SkeletonNode::new(2, Some(0), 1.0),
        SkeletonNode::new(3, Some(1), 1.0),
    ];
    for i in 0..4u32 {
        nodes.push(SkeletonNode::new(
            4 + i,
            Some(if i < 2 { 0 } else { 1 }),
            0.6,
        ));
    }
    let s = Skeleton { nodes };
    let out = pack(&s, 1.0, 1.0, 2026, 8);
    assert_eq!(out.len(), MAX_CANDIDATES);
    // 頭と尾は 1.0+0.3+1.0=2.3 離す必要があり、対角√2から上界は約0.61。
    assert!(out[0].scale >= 0.30, "縮尺が小さすぎる: {}", out[0].scale);
    assert!(max_violation(&s, &out[0], 1.0, 1.0) <= PACK_TOL);
}
