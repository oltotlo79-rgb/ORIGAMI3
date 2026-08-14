//! 充填の性能確認と配置品質の基準測定。

use ori3_propose::packing::{MAX_CANDIDATES, PACK_TOL, Packing};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use std::collections::BTreeSet;
use std::time::Instant;

const CENTER_DUPLICATE_TOL: f64 = 1e-7;

fn star(n: u32, len: f64) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), len));
    }
    Skeleton { nodes }
}

fn packing_is_complete_and_finite(output: &[Packing], expected_candidates: usize) -> bool {
    output.len() == expected_candidates
        && output.iter().all(|candidate| {
            let ids: BTreeSet<u32> = candidate.centers.iter().map(|(id, _)| *id).collect();
            candidate.scale.is_finite()
                && candidate.scale > 0.0
                && candidate.violation.is_finite()
                && candidate.violation <= PACK_TOL
                && candidate.centers.len() == 12
                && ids == (1..=12).collect()
                && candidate
                    .centers
                    .iter()
                    .all(|(_, center)| center[0].is_finite() && center[1].is_finite())
        })
}

fn invalid_output_description(output: &[Packing], expected_candidates: usize) -> String {
    let mut issues = Vec::new();
    if output.len() != expected_candidates {
        issues.push(format!("候補数{}(期待{expected_candidates})", output.len()));
    }
    for (index, candidate) in output.iter().enumerate() {
        if !candidate.scale.is_finite() || candidate.scale <= 0.0 {
            issues.push(format!("候補{index}の縮尺"));
        }
        if !candidate.violation.is_finite() || candidate.violation > PACK_TOL {
            issues.push(format!("候補{index}の違反量"));
        }
        let ids: BTreeSet<u32> = candidate.centers.iter().map(|(id, _)| *id).collect();
        if candidate.centers.len() != 12 || ids != (1..=12).collect() {
            issues.push(format!("候補{index}の葉ID/中心"));
        }
        if candidate
            .centers
            .iter()
            .any(|(_, center)| !center[0].is_finite() || !center[1].is_finite())
        {
            issues.push(format!("候補{index}の中心座標"));
        }
    }
    issues.join(", ")
}

/// 葉IDで対応付けた全中心のユークリッド距離が許容値以下なら同じ候補とする。
fn same_centers(a: &Packing, b: &Packing) -> bool {
    a.centers.len() == b.centers.len()
        && a.centers.iter().all(|(id, center_a)| {
            b.centers
                .iter()
                .find(|(other_id, _)| other_id == id)
                .is_some_and(|(_, center_b)| {
                    (center_a[0] - center_b[0]).hypot(center_a[1] - center_b[1])
                        <= CENTER_DUPLICATE_TOL
                })
        })
}

/// ソート済み標本のnearest-rank分位点。p50は500番目、p95は950番目を使う。
fn nearest_rank(sorted: &[f64], numerator: usize, denominator: usize) -> f64 {
    assert!(!sorted.is_empty() && numerator > 0 && numerator <= denominator);
    let rank = (sorted.len() * numerator).div_ceil(denominator);
    sorted[rank - 1]
}

/// 作業6の再現用測定。1テスト内で1,005回を直列実行し、全件を測ってから判定する。
/// 手元で145.21秒かかる測定専用テストで、結果は `docs/progress.md` に記録済み。
/// 実行: `cargo test -p ori3-propose --test perf_packing -- --ignored`
#[test]
#[ignore = "手元で145.21秒かかる測定専用テストのため、明示的に再測定するときだけ実行する"]
fn packing_quality_baseline_1005_runs() {
    let skeleton = star(12, 1.0);
    let mut run_count = 0usize;
    let mut finite_run_count = 0usize;
    let mut candidate_count = 0usize;
    let mut missing_output_count = 0usize;
    let mut invalid_runs = Vec::new();
    let mut configurations = BTreeSet::new();
    let mut seed_one_eight_starts = None;
    let mut starts_elapsed_ms = 0.0;
    let mut eight_starts_elapsed_ms = None;

    eprintln!("PACKING_BASELINE starts_comparison_begin");
    for starts in [1usize, 8, 16, 32, 64] {
        let started = Instant::now();
        let output = ori3_propose::pack(&skeleton, 1.0, 1.0, 1, starts);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        run_count += 1;
        configurations.insert((1u64, starts));
        missing_output_count += usize::from(output.is_empty());
        let expected_candidates = starts.min(MAX_CANDIDATES);
        if packing_is_complete_and_finite(&output, expected_candidates) {
            finite_run_count += 1;
        } else {
            invalid_runs.push(format!(
                "seed=1 starts={starts}: {}",
                invalid_output_description(&output, expected_candidates)
            ));
        }
        candidate_count += output.len();
        starts_elapsed_ms += elapsed_ms;
        let max_violation = output
            .iter()
            .map(|candidate| candidate.violation)
            .fold(0.0_f64, f64::max);
        let best_scale = output.first().map_or(f64::NAN, |candidate| candidate.scale);
        eprintln!(
            "PACKING_BASELINE start seed=1 starts={starts} candidates={} best_scale={best_scale:.15} elapsed_ms={elapsed_ms:.6} max_violation={max_violation:.15e}",
            output.len()
        );
        if starts == 8 {
            seed_one_eight_starts = Some(output.clone());
            eight_starts_elapsed_ms = Some(elapsed_ms);
        }
    }

    let mut best_scales = Vec::with_capacity(1_000);
    let mut elapsed_times_ms = Vec::with_capacity(1_000);
    let mut duplicate_pairs = 0usize;
    let mut candidate_pairs = 0usize;
    let mut redundant_candidates = 0usize;
    let mut runs_with_duplicates = 0usize;
    let mut maximum_violation = 0.0_f64;
    let mut seed_finite_run_count = 0usize;
    let mut repeated_configuration_matches = false;
    let seed_sweep_started = Instant::now();
    for seed in 0..1_000u64 {
        let started = Instant::now();
        let output = ori3_propose::pack(&skeleton, 1.0, 1.0, seed, 8);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        run_count += 1;
        configurations.insert((seed, 8usize));
        missing_output_count += usize::from(output.is_empty());
        if packing_is_complete_and_finite(&output, MAX_CANDIDATES) {
            finite_run_count += 1;
            seed_finite_run_count += 1;
        } else {
            invalid_runs.push(format!(
                "seed={seed} starts=8: {}",
                invalid_output_description(&output, MAX_CANDIDATES)
            ));
        }
        candidate_count += output.len();
        if seed == 1 {
            repeated_configuration_matches = seed_one_eight_starts.as_ref() == Some(&output);
        }

        if let Some(candidate) = output.first()
            && candidate.scale.is_finite()
        {
            best_scales.push(candidate.scale);
        }
        elapsed_times_ms.push(elapsed_ms);
        maximum_violation = output
            .iter()
            .map(|candidate| candidate.violation)
            .fold(maximum_violation, f64::max);

        let mut run_duplicate_pairs = 0usize;
        for i in 0..output.len() {
            if (0..i).any(|j| same_centers(&output[i], &output[j])) {
                redundant_candidates += 1;
            }
            for j in (i + 1)..output.len() {
                candidate_pairs += 1;
                if same_centers(&output[i], &output[j]) {
                    duplicate_pairs += 1;
                    run_duplicate_pairs += 1;
                }
            }
        }
        if run_duplicate_pairs > 0 {
            runs_with_duplicates += 1;
        }
    }
    let seed_sweep_wall_ms = seed_sweep_started.elapsed().as_secs_f64() * 1000.0;

    best_scales.sort_by(f64::total_cmp);
    elapsed_times_ms.sort_by(f64::total_cmp);
    let duplicate_pair_rate = duplicate_pairs as f64 / candidate_pairs as f64;
    let redundant_candidate_rate = redundant_candidates as f64 / 4_000.0;
    let runs_with_duplicates_rate = runs_with_duplicates as f64 / 1_000.0;
    let measured_pack_time_ms = starts_elapsed_ms + elapsed_times_ms.iter().sum::<f64>();

    if best_scales.len() == 1_000 {
        eprintln!(
            "PACKING_BASELINE seeds runs=1000 starts=8 finite_runs={seed_finite_run_count} missing={missing_output_count} scale_min={:.15} scale_p50={:.15} scale_p95={:.15} scale_max={:.15} time_total_ms={seed_sweep_wall_ms:.6} time_min_ms={:.6} time_p50_ms={:.6} time_p95_ms={:.6} time_max_ms={:.6} max_violation={maximum_violation:.15e}",
            best_scales[0],
            nearest_rank(&best_scales, 50, 100),
            nearest_rank(&best_scales, 95, 100),
            best_scales[999],
            elapsed_times_ms[0],
            nearest_rank(&elapsed_times_ms, 50, 100),
            nearest_rank(&elapsed_times_ms, 95, 100),
            elapsed_times_ms[999]
        );
    } else {
        eprintln!(
            "PACKING_BASELINE seeds runs=1000 starts=8 finite_runs={seed_finite_run_count} missing={missing_output_count} scale_values={} time_total_ms={seed_sweep_wall_ms:.6} max_violation={maximum_violation:.15e}",
            best_scales.len()
        );
    }
    eprintln!(
        "PACKING_BASELINE duplicates tolerance={CENTER_DUPLICATE_TOL:.1e} duplicate_pairs={duplicate_pairs}/{candidate_pairs} pair_rate={duplicate_pair_rate:.15} redundant_candidates={redundant_candidates}/4000 redundant_rate={redundant_candidate_rate:.15} runs_with_duplicates={runs_with_duplicates}/1000 run_rate={runs_with_duplicates_rate:.15}"
    );
    eprintln!(
        "PACKING_BASELINE complete runs={run_count} unique_parameter_pairs={} candidates={candidate_count} finite_runs={finite_run_count} missing={missing_output_count} measured_pack_time_ms={measured_pack_time_ms:.6}",
        configurations.len()
    );

    assert_eq!(run_count, 1_005);
    assert_eq!(configurations.len(), 1_004);
    assert_eq!(finite_run_count, 1_005);
    assert_eq!(missing_output_count, 0);
    assert!(invalid_runs.is_empty(), "不正な実行: {invalid_runs:?}");
    assert_eq!(candidate_count, 4_017);
    assert_eq!(candidate_pairs, 6_000);
    assert!(repeated_configuration_matches);
    assert!(
        eight_starts_elapsed_ms.is_some_and(|elapsed_ms| elapsed_ms < 1_000.0),
        "12葉・8スタートが既存の1秒上限を超えた: {eight_starts_elapsed_ms:?}ms"
    );
    assert_eq!(best_scales.len(), 1_000);
    assert!(best_scales.iter().all(|value| value.is_finite()));
    assert!(elapsed_times_ms.iter().all(|value| value.is_finite()));
}
