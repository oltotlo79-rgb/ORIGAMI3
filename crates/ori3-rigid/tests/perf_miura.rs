//! Task 2-0: 大規模CPでの性能回帰テスト(NFR-002: 面400・辺1,000規模で
//! solve 33ms以内)。
//!
//! 20×20のミウラ折りパターン(面400・頂点441・辺840・ヒンジ760・基本ループ361)
//! を構築し、スライダー操作相当の使い方(前回解をwarm startにdriver角を2°ずつ
//! 進める)でのsolve 1回あたりの時間を測る。
//!
//! # 計測方法と結果の記録
//!
//! - `#[ignore]`なしの通常テストとして計算結果の正しさを確かめ、実時間の上限は
//!   releaseビルドでだけ判定する。通常の`cargo test --workspace`で実時間を
//!   合否にすると、最適化なしの速さと計算機の混み具合を測ってしまうためである。
//!   releaseでは同じ系列を3回測り、一番良かった回で判定する。OSが一時的に
//!   止めた時間を1回きりの判定へ混ぜないためで、3回とも遅ければ性能後退は
//!   捕まる。通常ビルドは数値の正しさだけを見るので、従来どおり1回にする。
//! - 実測(2026-08-05, 開発機 Windows 11 / Task 2-0改修後):
//!   warm start 1回あたり debug 約85〜95ms / release 約2.7〜6.2ms
//!   (NFR-002の33msに対し5倍以上の余裕。反復は4〜5回)
//! - 改修前(M1時点のソルバー)を同一条件・releaseで実測すると1回あたり
//!   約27〜37秒(目標の約1,000倍)。律速は数値微分の全域再伝播を含む
//!   密ヤコビアン(m=4332×k=759)と、その正規方程式の密ガウス消去だった
//! - releaseの上限はNFR-002の33msと、`solve_near`の実測に余裕を取った100ms。
//!   接触診断込みの操作は既存の16ms枠を維持する。最適化ありの20回実測値と
//!   上限の比は次のとおり(2026-08-20、Windows 11開発機、20回連続、失敗0件)。
//!
//! | 対象 | 最大 | 中央 | 最小 | 上限 | 最大÷上限 |
//! |---|---:|---:|---:|---:|---:|
//! | warm start solve | 2.0420ms | 1.6485ms | 1.5559ms | 33ms | 0.0619 |
//! | solve_near | 9.4964ms | 8.6913ms | 8.1973ms | 100ms | 0.0950 |
//! | 接触診断込み solve_motion | 4.9204ms | 3.3999ms | 3.2059ms | 16ms | 0.3075 |
//!
//! どれも手元の最大値が上限の1/3以下である。CIは開発機より約3.6倍遅い実測が
//! あるため、実測値をそのまま上限にせず、余裕を取った。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::intersect::contact_scan_profile;
use ori3_rigid::{self_intersection_pairs, solve, solve_motion, solve_near};

/// releaseビルドでのsolve 1回あたりの上限(モジュールコメントの計測記録を参照)。
const SOLVE_BUDGET: Duration = Duration::from_millis(33);

/// `solve_near`(角度を次々に指定する使い方)1回あたりのrelease上限。
const NEAR_BUDGET: Duration = Duration::from_millis(100);
/// 接触診断込みの通常1フレームのrelease上限。
const MOTION_BUDGET: Duration = Duration::from_millis(16);

/// 実時間の上限は最適化ありの性能ジョブだけで判定する。
fn assert_within_release_budget(elapsed: Duration, budget: Duration, label: &str) {
    if cfg!(debug_assertions) {
        return;
    }
    assert!(
        elapsed < budget,
        "{label}: {elapsed:?}(上限 {budget:?}。モジュール冒頭の計測記録を参照)"
    );
}

/// nc×nr面のミウラ折りCP。頂点(i,j)は x=i·dx、y=(j+奇数列なら振れ幅s)·dy。
/// 縦線はまっすぐ(内部の山谷は列+行のパリティで交互)、横線はジグザグ
/// (内部は行ごとに山谷が交互)。各内部頂点は次数4で、縦2辺が一直線のため
/// 川崎の定理を自動的に満たし、山谷3:1で前川の定理も満たす(平坦折り可能)。
fn miura_cp(nc: usize, nr: usize) -> CreasePattern {
    let s = 0.35; // ジグザグの振れ幅(dy比)
    let dx = 1.0 / nc as f64;
    let dy = 1.0 / (nr as f64 + s);
    let vid = |i: usize, j: usize| (j * (nc + 1) + i) as u32;
    let mut vertices = Vec::new();
    for j in 0..=nr {
        for i in 0..=nc {
            vertices.push(Vertex {
                id: vid(i, j),
                pos: [
                    i as f64 * dx,
                    (j as f64 + if i % 2 == 1 { s } else { 0.0 }) * dy,
                ],
            });
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    // 縦の線分(辺ID = j*(nc+1)+i の順で採番される)
    for j in 0..nr {
        for i in 0..=nc {
            let kind = if i == 0 || i == nc {
                EdgeKind::Border
            } else if (i + j) % 2 == 0 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            edges.push(Edge {
                id: edges.len() as u32,
                v0: vid(i, j),
                v1: vid(i, j + 1),
                kind,
            });
        }
    }
    // 横のジグザグ線分
    for j in 0..=nr {
        for i in 0..nc {
            let kind = if j == 0 || j == nr {
                EdgeKind::Border
            } else if j % 2 == 1 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            edges.push(Edge {
                id: edges.len() as u32,
                v0: vid(i, j),
                v1: vid(i + 1, j),
                kind,
            });
        }
    }
    CreasePattern {
        next_vertex_id: vertices.len() as u32,
        next_edge_id: edges.len() as u32,
        vertices,
        edges,
    }
}

#[test]
fn miura_20x20_solve_stays_within_frame_budget() {
    let mut best = Duration::MAX;
    let passes = if cfg!(debug_assertions) { 1 } else { 3 };
    for pass in 1..=passes {
        let (nc, nr) = (20, 20);
        let cp = miura_cp(nc, nr);
        let faces = extract_faces(&cp);
        assert_eq!(faces.len(), 400);
        assert_eq!(cp.edges.len(), 840);

        // 中央付近の縦ヒンジ(山)をdriverにする。縦線分の辺IDは j*(nc+1)+i
        let hinge = (nr / 2 * (nc + 1) + nc / 2) as u32;
        assert_eq!(
            cp.edges[hinge as usize].kind,
            EdgeKind::Mountain,
            "駆動ヒンジは山折りのはず"
        );
        let drv = |deg: f64| {
            vec![Driver {
                hinge,
                target_angle_deg: deg,
            }]
        };

        // 冷間の初回solve(時間制限の対象外。収束は必須)
        let cold = solve(&cp, &faces, &drv(20.0), None);
        assert!(cold.converged, "iterations={}", cold.iterations);

        // スライダー操作相当: warm startで2°ずつ進め、1回ごとの時間を測る
        let mut prev = cold.angles;
        let mut worst = Duration::ZERO;
        for step in 1..=5 {
            let deg = 20.0 + 2.0 * f64::from(step);
            let t0 = Instant::now();
            let res = solve(&cp, &faces, &drv(deg), Some(&prev));
            let dt = t0.elapsed();
            assert!(res.converged, "step={step} iterations={}", res.iterations);
            println!(
                "{pass}回目 step={step} deg={deg} iterations={} time={dt:?}",
                res.iterations
            );
            worst = worst.max(dt);
            prev = res.angles;
        }
        println!("{pass}回目: warm start solve最悪={worst:?}");
        best = best.min(worst);
    }
    println!("warm start solve最良={best:?}(上限 {SOLVE_BUDGET:?})");
    assert_within_release_budget(best, SOLVE_BUDGET, "warm start solve");
}

/// 角度スライダーで折り角を次々に指定していく使い方の性能(NFR-002)。
/// いま操作している1本だけを固定し、以前の指定は目標として `solve_near` で
/// 追従させる。目標保持→閉包精密化の2段になるぶん通常solveより重い。
///
/// 実測(2026-08-07, 開発機 Windows 11): warm startありの1回あたり
/// debug 約306〜516ms / release 約8.8〜12.7ms(NFR-002の33msに対し2.5倍以上の
/// 余裕。反復は13〜16回)。warm startなしの初回は debug 720ms / release 20.5ms。
#[test]
fn miura_20x20_solve_near_stays_within_frame_budget() {
    let mut best = Duration::MAX;
    let passes = if cfg!(debug_assertions) { 1 } else { 3 };
    for pass in 1..=passes {
        let (nc, nr) = (20, 20);
        let cp = miura_cp(nc, nr);
        let faces = extract_faces(&cp);
        assert_eq!(faces.len(), 400);

        // 中央付近の縦ヒンジを左から5本、1本ずつ指定していく
        let picked: Vec<u32> = (0..5)
            .map(|k| (nr / 2 * (nc + 1) + nc / 2 + k) as u32)
            .collect();
        let goal = |e: u32| if e.is_multiple_of(2) { 24.0 } else { -24.0 };

        let mut warm: Option<HashMap<u32, f64>> = None;
        let mut worst = Duration::ZERO;
        for i in 1..=picked.len() {
            let h = picked[i - 1];
            let hard = vec![Driver {
                hinge: h,
                target_angle_deg: goal(h),
            }];
            let targets: HashMap<u32, f64> =
                picked[..i - 1].iter().map(|&e| (e, goal(e))).collect();
            let t0 = Instant::now();
            let res = solve_near(&cp, &faces, &hard, &targets, warm.as_ref());
            let dt = t0.elapsed();
            println!("{pass}回目 i={i} iterations={} time={dt:?}", res.iterations);
            assert!(res.converged, "i={i} iterations={}", res.iterations);
            // 1本目(warm startなし)は冷間なので時間制限の対象外
            if i > 1 {
                worst = worst.max(dt);
            }
            warm = Some(res.angles);
        }
        println!("{pass}回目: solve_near最悪={worst:?}");
        best = best.min(worst);
    }
    println!("solve_near最良={best:?}(上限 {NEAR_BUDGET:?})");
    assert_within_release_budget(best, NEAR_BUDGET, "solve_near");
}

#[test]
fn miura_20x20_contact_check_stays_within_frame_budget() {
    let (nc, nr) = (20, 20);
    let cp = miura_cp(nc, nr);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 400);
    let hinge = (nr / 2 * (nc + 1) + nc / 2) as u32;
    let start = solve(
        &cp,
        &faces,
        &[Driver {
            hinge,
            target_angle_deg: 20.0,
        }],
        None,
    );
    assert!(start.converged);

    let mut warm_drivers: Vec<Driver> = start
        .angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    warm_drivers.sort_unstable_by_key(|driver| driver.hinge);
    let stage_started = Instant::now();
    let warm_pose = solve(&cp, &faces, &warm_drivers, Some(&start.angles));
    let warm_pose_time = stage_started.elapsed();
    let stage_started = Instant::now();
    let requested = solve(
        &cp,
        &faces,
        &[Driver {
            hinge,
            target_angle_deg: 22.0,
        }],
        Some(&start.angles),
    );
    let requested_solve_time = stage_started.elapsed();
    assert!(warm_pose.converged);
    assert!(requested.converged);
    let warm_scan = contact_scan_profile(&warm_pose.frame);
    let requested_scan = contact_scan_profile(&requested.frame);
    println!(
        "接触診断・段階内訳(時間枠外): warm_pose={warm_pose_time:?} requested_solve={requested_solve_time:?}"
    );
    println!("接触診断・開始姿勢走査: {warm_scan:?}");
    println!("接触診断・要求姿勢走査: {requested_scan:?}");
    println!("接触診断・solve_motion走査回数: 2 (開始姿勢1 + 要求姿勢1)");

    let mut best = Duration::MAX;
    let mut first_motion = None;
    let passes = if cfg!(debug_assertions) { 1 } else { 3 };
    for pass in 1..=passes {
        let t0 = Instant::now();
        let motion = solve_motion(
            &cp,
            &faces,
            &[Driver {
                hinge,
                target_angle_deg: 22.0,
            }],
            None,
            Some(&start.angles),
            true,
        );
        let elapsed = t0.elapsed();
        println!(
            "{pass}回目 面400・接触診断: iterations={} time={elapsed:?}",
            motion.result.iterations
        );
        assert!(motion.result.converged);
        assert!(!motion.contact_detected);
        best = best.min(elapsed);
        if first_motion.is_none() {
            first_motion = Some(motion);
        }
    }
    println!("面400・接触診断最良={best:?}(上限 {MOTION_BUDGET:?})");
    assert_within_release_budget(best, MOTION_BUDGET, "接触診断込みの追従");
    let motion = first_motion.expect("少なくとも1回の接触診断を実行する");

    // 性能変更後も同一入力の交差集合と数値結果が毎回同一であることを、
    // 時間枠の外で確認する（SYS-004）。この姿勢の従来交差集合は空。
    let expected_pairs = self_intersection_pairs(&motion.result.frame);
    assert!(expected_pairs.is_empty());
    for _ in 0..5 {
        let repeated = solve_motion(
            &cp,
            &faces,
            &[Driver {
                hinge,
                target_angle_deg: 22.0,
            }],
            None,
            Some(&start.angles),
            true,
        );
        assert_eq!(repeated.result.angles, motion.result.angles);
        assert_eq!(
            repeated.result.closure_rms.to_bits(),
            motion.result.closure_rms.to_bits()
        );
        assert_eq!(repeated.result.iterations, motion.result.iterations);
        assert_eq!(repeated.contact_detected, motion.contact_detected);
        assert_eq!(
            self_intersection_pairs(&repeated.result.frame),
            expected_pairs
        );
        for (actual, expected) in repeated
            .result
            .frame
            .faces
            .iter()
            .zip(&motion.result.frame.faces)
        {
            assert_eq!(actual.face, expected.face);
            assert_eq!(actual.layer, expected.layer);
            assert_eq!(actual.polygon.len(), expected.polygon.len());
            for (actual, expected) in actual.polygon.iter().zip(&expected.polygon) {
                assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
}
