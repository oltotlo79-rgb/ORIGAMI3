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

/// 同じ入力の再計算が浮動小数点のビットまで同じかを、整数だけで比べる。
/// 固定した計算値との比較ではなく、最初の1回を残り9回の基準にする。
fn packing_bits(output: &[Packing]) -> Vec<u64> {
    let mut bits = vec![output.len() as u64];
    for candidate in output {
        bits.extend([
            candidate.scale.to_bits(),
            candidate.violation.to_bits(),
            candidate.centers.len() as u64,
        ]);
        for &(leaf_id, center) in &candidate.centers {
            bits.extend([u64::from(leaf_id), center[0].to_bits(), center[1].to_bits()]);
        }
        bits.push(candidate.circles.len() as u64);
        for circle in &candidate.circles {
            bits.extend([
                u64::from(circle.leaf_id),
                circle.circle_index as u64,
                circle.center[0].to_bits(),
                circle.center[1].to_bits(),
                circle.radius.to_bits(),
            ]);
        }
    }
    bits
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

/// 中心だけを紙内に置く現行制約で、12葉がこの縮尺を満たせることの独立検算。
#[test]
fn twelve_leaf_center_containment_lower_bound_is_feasible() {
    const LOWER_BOUND: f64 = 0.194_277_036;
    let s = star(12, 1.0);
    let centers = [
        [0.600_398_070, 1.0],
        [0.0, 0.0],
        [0.399_425_171, 0.0],
        [0.800_204_016, 0.0],
        [1.0, 0.333_299_635],
        [0.600_212_313, 0.333_192_196],
        [0.400_706_494, 0.666_632_833],
        [0.200_033_273, 0.333_492_338],
        [1.0, 1.0],
        [0.0, 0.667_164_704],
        [0.800_087_884, 0.666_632_454],
        [0.200_674_750, 1.0],
    ];
    let packing = Packing {
        scale: LOWER_BOUND,
        centers: (1..=12).zip(centers).collect(),
        violation: 0.0,
        circles: Vec::new(),
    };

    let violation = max_violation(&s, &packing, 1.0, 1.0);
    assert_eq!(
        violation, 0.0,
        "中心包含の12葉下限{LOWER_BOUND}を満たさない"
    );
}

/// 作業8: 作業6で実在を確認した下限へ探索自身が到達し、10回とも同じ結果になる。
/// 旧debug壁時計条件は、`perf_packing.rs` の
/// `work8_twelve_leaf_eight_starts_ten_runs_stay_within_release_budget` が
/// release 10回の最大値を同じ1秒上限で判定する。
#[test]
fn work8_twelve_leaf_search_reaches_the_known_feasible_scale_ten_times() {
    const REQUIRED_SCALE: f64 = 0.194_277_036;
    const MAX_VIOLATION: f64 = 1e-9;
    const RUNS: usize = 10;

    let skeleton = star(12, 1.0);
    let mut expected_bits = None;
    let mut passed = 0;
    let mut achieved = (f64::NAN, f64::NAN, f64::NAN);
    for run in 1..=RUNS {
        let output = pack(&skeleton, 1.0, 1.0, 1, 8);
        assert_eq!(output.len(), MAX_CANDIDATES, "{run}回目の候補数");
        let best = &output[0];
        assert!(
            best.scale.is_finite() && best.scale >= REQUIRED_SCALE,
            "{run}回目の縮尺{}が下限{REQUIRED_SCALE}へ届かない",
            best.scale
        );
        assert!(
            best.violation.is_finite() && best.violation <= MAX_VIOLATION,
            "{run}回目の報告違反量{}",
            best.violation
        );
        let checked_violation = max_violation(&skeleton, best, 1.0, 1.0);
        if run == 1 {
            achieved = (best.scale, best.violation, checked_violation);
        }
        assert!(
            checked_violation <= MAX_VIOLATION,
            "{run}回目の独立検算違反量{checked_violation}"
        );
        assert_eq!(best.centers.len(), 12, "{run}回目の中心数");
        assert_eq!(best.circles.len(), 12, "{run}回目の円数");
        assert!(
            best.centers
                .iter()
                .enumerate()
                .all(|(index, (leaf_id, center))| {
                    *leaf_id == index as u32 + 1 && center[0].is_finite() && center[1].is_finite()
                }),
            "{run}回目の葉IDまたは中心座標が不正"
        );

        let actual_bits = packing_bits(&output);
        if let Some(expected) = &expected_bits {
            assert_eq!(&actual_bits, expected, "{run}回目が初回とbit不一致");
        } else {
            expected_bits = Some(actual_bits);
        }
        passed += 1;
    }
    assert_eq!(passed, RUNS);
    println!(
        "作業8: scale={:.15}, violation={:.15e}, checked={:.15e}, 決定性={passed}/{RUNS}",
        achieved.0, achieved.1, achieved.2
    );
}

/// 半径まで紙内に収める案B(円全体包含)の独立検算: 4×3格子なら12葉でこの
/// 縮尺を満たせる。
///
/// **2026-08-16判断4により、紙内包含は案A(円の中心が紙内)に決定し、案Bは
/// 不採用にした。** このテストは「採らなかった案でも配置は成立する」という
/// 記録として残す。ここで使う`0.124999999`は4×3格子1つを検算しただけの
/// 下限であり、最良値ではない。`scratchpad/containment-report.md`の本測定
/// (starts=8のマルチスタート探索)では、この骨格で実測`0.139958812`
/// (この格子下限の1.1197倍)まで届いている。
#[test]
fn twelve_leaf_full_circle_grid_lower_bound_is_feasible() {
    const LOWER_BOUND: f64 = 0.124_999_999;
    let s = star(12, 1.0);
    let mut centers = Vec::with_capacity(12);
    for row in 0..3 {
        for column in 0..4 {
            let id = 1 + row * 4 + column;
            centers.push((
                id,
                [
                    0.125 + f64::from(column) * 0.25,
                    0.125 + f64::from(row) * 0.375,
                ],
            ));
        }
    }

    for (index, &(id_a, a)) in centers.iter().enumerate() {
        let radius = LOWER_BOUND * s.leaf_radius(id_a);
        let boundary_margin = a[0].min(1.0 - a[0]).min(a[1]).min(1.0 - a[1]);
        assert!(
            boundary_margin >= radius,
            "葉{id_a}の円が紙からはみ出す: margin={boundary_margin}, radius={radius}"
        );
        for &(id_b, b) in &centers[index + 1..] {
            let distance = (a[0] - b[0]).hypot(a[1] - b[1]);
            let required = LOWER_BOUND * s.leaf_distance(id_a, id_b);
            assert!(
                distance >= required,
                "葉{id_a}と葉{id_b}が重なる: distance={distance}, required={required}"
            );
        }
    }
}
