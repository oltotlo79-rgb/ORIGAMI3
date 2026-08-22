//! 作業24〜26: 実作品の独立した完成形に対する終点測定と完成探索。
//!
//! 標本は折り鶴・やっこさん・鳥の基本形の3件。完成目標は探索結果から作らず、
//! 各作品の既存受け入れ手順を最後まで折った参照形から先に固定している。

use ori3_model::{CreasePattern, Document, EdgeKind, Paper};
use ori3_propose::enumerate::{FoldSession, MAX_SEAM_GAP, PoseScan, Unverified};
use ori3_propose::finish::{FinishGaps, FinishTarget, TargetTip};
use ori3_propose::search::{
    CompletionTolerance, FoldGoal, GapWeights, SearchBudget, SearchOutcome, SearchStop, TipSite,
    search_to_completion, search_to_finish,
};
use ori3_propose::skeleton::TipPos2d;
use ori3_propose::verify::{VerifiedPlan, verify_search_completion, verify_search_outcome};

struct Sample {
    name: &'static str,
    document: Document,
    goal: FoldGoal,
    measured_task_24: FinishGaps,
}

const RUNS: usize = 10;

/// 部分集合候補を加えた実測から、改善量のおよそ80%を恒久に守る境目。
///
/// 実測値そのもの（折り鶴0.172464、鳥の基本形は7e-16未満）を境目にはせず、
/// 変更前との差の約20%を回帰余裕として残す。
const BIRD_WIDTH_REGRESSION_LIMIT: f64 = 0.34;

/// 記録した作業24の小数との照合差。
///
/// 完成参照との数値残差は最大`1.77e-15`、完成/未完成を分ける
/// 最小差は、やっこさんの太さ `0.3106601718 - 0.2485281374 = 0.0621320344`。
/// `1e-9` は揺れより5桁以上粗く、形の差より7桁以上細い。作業25の完成許容値
/// ではなく、計算した小数を記録と厳密一致させないためだけの照合差である。
const MEASUREMENT_TOL: f64 = 1e-9;

fn square_document() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn fixture_document(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let cp: CreasePattern =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"));
    let mut document = square_document();
    document.cp = cp;
    document
}

fn target_tip(leaf_id: u32, length: f64, width: f64, position: [f64; 2]) -> TargetTip {
    TargetTip {
        leaf_id,
        length,
        width,
        pos: Some(TipPos2d::new(position[0], position[1])),
    }
}

fn corner_sites() -> Vec<TipSite> {
    [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        .into_iter()
        .enumerate()
        .map(|(index, material)| TipSite {
            leaf_id: index as u32 + 1,
            material,
        })
        .collect()
}

/// `ori3-layers/tests/acceptance_crane.rs::crane` の11手を最後まで折った参照形を
/// 2026-08-20に測定して固定した目標。探索用CPだけを読み、探索終点からは作らない。
fn crane_sample() -> Sample {
    let target = FinishTarget {
        tips: vec![
            target_tip(
                1,
                0.790_569_415_042_094_2,
                1.669_023_860_241_399_8,
                [0.816_987_298_107_780_2, -0.049_038_105_676_658_35],
            ),
            target_tip(
                2,
                0.317_157_287_525_381_2,
                0.517_638_090_205_052_7,
                [-0.232_175_248_459_309_2, -0.232_175_248_459_310_76],
            ),
            target_tip(
                3,
                1.0,
                0.517_638_090_205_050_6,
                [0.267_949_192_431_123_36, 1.0],
            ),
            target_tip(
                4,
                0.317_157_287_525_380_9,
                0.517_638_090_205_040_7,
                [-0.232_175_248_459_311_95, -0.232_175_248_459_307_62],
            ),
        ],
    };
    Sample {
        name: "折り鶴",
        document: fixture_document("cp-crane.json"),
        goal: FoldGoal {
            target,
            body: [0.5, 0.5],
            sites: corner_sites(),
        },
        measured_task_24: FinishGaps {
            count: 0.0,
            length: 1.142_732_609_721_514_7,
            width: 1.337_205_669_355_358_5,
            position: 0.432_647_886_031_762_2,
        },
    }
}

fn yakko_document() -> Document {
    let mut document = square_document();
    let cp = &mut document.cp;
    let (m1, m2, m3, m4) = ([0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]);
    for (a, b) in [(m1, m2), (m2, m3), (m3, m4), (m4, m1)] {
        ori3_cp::insert_segment(cp, a, b, EdgeKind::Valley);
    }
    for t in [0.25, 0.75] {
        for (a, b, kind) in [
            ([t, 0.0], [t, 0.25], EdgeKind::Mountain),
            ([t, 0.25], [t, 0.75], EdgeKind::Valley),
            ([t, 0.75], [t, 1.0], EdgeKind::Mountain),
            ([0.0, t], [0.25, t], EdgeKind::Mountain),
            ([0.25, t], [0.75, t], EdgeKind::Valley),
            ([0.75, t], [1.0, t], EdgeKind::Mountain),
        ] {
            ori3_cp::insert_segment(cp, a, b, kind);
        }
    }
    document
}

/// `ori3-rigid/tests/acceptance_yakko.rs` と同じ全20ヒンジ±180°の完成形。
/// 内部4点が完成正方形の4隅へ1対1に写るという理論値も同テストが検証している。
fn yakko_sample() -> Sample {
    let materials = [[0.75, 0.25], [0.75, 0.75], [0.25, 0.75], [0.25, 0.25]];
    let positions = [[1.0, -1.0], [-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
    Sample {
        name: "やっこさん",
        document: yakko_document(),
        goal: FoldGoal {
            target: FinishTarget {
                tips: (0..4)
                    .map(|index| {
                        target_tip(
                            index as u32 + 1,
                            1.0,
                            std::f64::consts::SQRT_2,
                            positions[index],
                        )
                    })
                    .collect(),
            },
            body: [0.5, 0.5],
            sites: materials
                .into_iter()
                .enumerate()
                .map(|(index, material)| TipSite {
                    leaf_id: index as u32 + 1,
                    material,
                })
                .collect(),
        },
        measured_task_24: FinishGaps {
            count: 0.0,
            length: 0.000_000_000_000_000_666_133_814_775_093_9,
            width: 0.310_660_171_779_821_36,
            position: 0.000_000_000_000_001_076_023_672_781_104_9,
        },
    }
}

/// `ori3-layers/tests/acceptance_crane.rs::bird_base` の6手を最後まで折った参照形を
/// 2026-08-20に測定して固定した目標と、その時点の独立CP fixture。
fn bird_base_sample() -> Sample {
    let target = FinishTarget {
        tips: vec![
            target_tip(
                1,
                0.414_213_562_373_094_9,
                0.517_638_090_205_041_7,
                [-0.000_000_000_000_000_08, -0.414_213_562_373_094_9],
            ),
            target_tip(
                2,
                1.0,
                0.517_638_090_205_042_9,
                [-0.000_000_000_000_000_08, 1.0],
            ),
            target_tip(
                3,
                0.414_213_562_373_094_65,
                0.517_638_090_205_045,
                [0.000_000_000_000_000_86, -0.414_213_562_373_094_65],
            ),
            target_tip(
                4,
                1.0,
                0.517_638_090_205_041_3,
                [-0.000_000_000_000_000_08, 1.0],
            ),
        ],
    };
    Sample {
        name: "鳥の基本形",
        document: fixture_document("cp-bird-base.json"),
        goal: FoldGoal {
            target,
            body: [0.5, 0.5],
            sites: corner_sites(),
        },
        measured_task_24: FinishGaps {
            count: 0.0,
            length: 0.707_106_781_186_548_3,
            width: 1.732_050_807_568_870_5,
            position: 0.482_962_913_144_534_05,
        },
    }
}

fn samples() -> Vec<Sample> {
    vec![crane_sample(), yakko_sample(), bird_base_sample()]
}

fn assert_gaps_near(name: &str, got: FinishGaps, want: FinishGaps) -> f64 {
    let mut max_delta = 0.0_f64;
    for (measure, got, want) in [
        ("角の数", got.count, want.count),
        ("長さ", got.length, want.length),
        ("太さ", got.width, want.width),
        ("位置", got.position, want.position),
    ] {
        let delta = (got - want).abs();
        assert!(
            delta <= MEASUREMENT_TOL,
            "{name}: {measure}の実測{got:.12}が記録{want:.12}から{MEASUREMENT_TOL:.1e}より大きく動いた"
        );
        max_delta = max_delta.max(delta);
    }
    max_delta
}

fn assert_scalar_near(name: &str, field: &str, got: f64, want: f64) -> f64 {
    let delta = (got - want).abs();
    assert!(
        delta <= MEASUREMENT_TOL,
        "{name}: {field}の{RUNS}回比較差|{got} - {want}|が{MEASUREMENT_TOL:.1e}を超えた"
    );
    delta
}

/// 決定性の離散部分は完全一致、小数は§10.7.7に従って実測由来の許容差で比べる。
fn assert_outcome_deterministic(name: &str, got: &SearchOutcome, want: &SearchOutcome) -> f64 {
    assert_eq!(got.stop, want.stop, "{name}: 停止理由が一致しない");
    assert_eq!(
        got.states_expanded, want.states_expanded,
        "{name}: 展開数が一致しない"
    );
    assert_eq!(
        got.states_generated, want.states_generated,
        "{name}: 生成数が一致しない"
    );
    assert_eq!(
        got.max_branching, want.max_branching,
        "{name}: 最大分岐が一致しない"
    );
    assert_eq!(
        got.depth_capped, want.depth_capped,
        "{name}: 深さ打切り数が一致しない"
    );
    assert_eq!(
        got.steps.len(),
        want.steps.len(),
        "{name}: 手数が一致しない"
    );
    let mut max_delta = assert_gaps_near(name, got.start_gaps, want.start_gaps);
    max_delta = max_delta.max(assert_gaps_near(name, got.best_gaps, want.best_gaps));
    max_delta = max_delta.max(assert_scalar_near(
        name,
        "開始点数",
        got.start_score,
        want.start_score,
    ));
    max_delta = max_delta.max(assert_scalar_near(
        name,
        "終点点数",
        got.best_score,
        want.best_score,
    ));
    for (index, (got, want)) in got.steps.iter().zip(&want.steps).enumerate() {
        let step = format!("{}手目", index + 1);
        assert_eq!(got.mv.id, want.mv.id, "{name}: {step}のIDが一致しない");
        assert_eq!(
            got.mv.closes, want.mv.closes,
            "{name}: {step}の閉じる線が一致しない"
        );
        assert_eq!(
            got.mv.mask, want.mv.mask,
            "{name}: {step}のmaskが一致しない"
        );
        assert_eq!(
            got.mv.penetrations, want.mv.penetrations,
            "{name}: {step}のめり込み数が一致しない"
        );
        assert_eq!(
            got.mv.poses_checked, want.mv.poses_checked,
            "{name}: {step}の姿勢数が一致しない"
        );
        for point in 0..2 {
            for axis in 0..2 {
                max_delta = max_delta.max(assert_scalar_near(
                    name,
                    &format!("{step}の直線[{point}][{axis}]"),
                    got.mv.line[point][axis],
                    want.mv.line[point][axis],
                ));
            }
        }
        max_delta = max_delta.max(assert_scalar_near(
            name,
            &format!("{step}の裂け"),
            got.mv.max_seam_gap,
            want.mv.max_seam_gap,
        ));
        max_delta = max_delta.max(assert_gaps_near(name, got.gaps, want.gaps));
        max_delta = max_delta.max(assert_scalar_near(
            name,
            &format!("{step}の点数"),
            got.score,
            want.score,
        ));
    }
    max_delta
}

/// 作業24: 許容値による合否を一切使わず、探索終点の4値と全姿勢の健全性を測る。
#[test]
fn task_24_measures_actual_completion_gaps_before_setting_tolerances() {
    for sample in samples() {
        let session = FoldSession::new(&sample.document)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.name));
        let started = std::time::Instant::now();
        let outcome = search_to_finish(
            &session,
            &sample.goal,
            GapWeights::DEFAULT,
            SearchBudget::DEFAULT,
        );
        let report = verify_search_outcome(
            &session,
            &outcome,
            &sample.goal,
            GapWeights::DEFAULT,
            PoseScan::DEFAULT,
        );
        let ids: Vec<_> = outcome.steps.iter().map(|step| step.mv.id).collect();
        println!(
            "TASK24 {} start={:?} final={:?} ids={ids:?} stop={:?} states={}/{} branch={} elapsed={:.3}s {}",
            sample.name,
            report.start_gaps,
            report.final_gaps,
            outcome.stop,
            outcome.states_expanded,
            outcome.states_generated,
            outcome.max_branching,
            started.elapsed().as_secs_f64(),
            report.describe(),
        );
        assert!(
            report.passed(),
            "{}: 探索手順を最後まで安全に折れない: {report:?}",
            sample.name
        );
        assert_gaps_near(sample.name, report.final_gaps, sample.measured_task_24);
    }
}

#[test]
fn task_25_uses_eighty_percent_headroom_from_task_24_measurements() {
    let tolerance = CompletionTolerance::DEFAULT;
    for (measure, limit, actual) in [
        ("角の数", tolerance.count, 1.0 / 12.0),
        (
            "長さ",
            tolerance.length,
            bird_base_sample().measured_task_24.length,
        ),
        (
            "太さ",
            tolerance.width,
            yakko_sample().measured_task_24.width,
        ),
        (
            "位置",
            tolerance.position,
            crane_sample().measured_task_24.position,
        ),
    ] {
        let ratio = limit / actual;
        assert!(
            (ratio - 0.8).abs() <= MEASUREMENT_TOL,
            "{measure}: 許容{limit:.12} / 実測・離散根拠{actual:.12} = {ratio:.12} が80%でない"
        );
    }

    assert!(!tolerance.contains(&FinishGaps {
        count: 1.0 / 12.0,
        length: 0.0,
        width: 0.0,
        position: 0.0,
    }));
    for sample in samples() {
        assert!(
            !tolerance.contains(&sample.measured_task_24),
            "{}: 作業24の未完成終点を完成扱いした",
            sample.name
        );
    }
}

fn run_to_completion(sample: &Sample) -> SearchOutcome {
    let session = FoldSession::new(&sample.document)
        .unwrap_or_else(|error| panic!("{}: {error}", sample.name));
    search_to_completion(
        &session,
        &sample.goal,
        GapWeights::DEFAULT,
        SearchBudget::DEFAULT,
        CompletionTolerance::DEFAULT,
    )
}

/// 同じ3標本を完成許容値まで探し、部分集合候補・全手順・10回の決定性を確認する。
#[test]
fn completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten() {
    let mut completed = 0usize;
    let mut failed = Vec::new();
    let mut typed_states = 0usize;
    let budget = SearchBudget::DEFAULT;
    assert_eq!(budget.max_states, 12, "状態数上限を緩めた");
    assert_eq!(budget.branch, 3, "保持する分岐数上限を緩めた");
    assert_eq!(budget.max_depth, 8, "深さ上限を緩めた");
    for sample in samples() {
        let sample_started = std::time::Instant::now();
        // 10回とも探索を最初から直列に計算する。他担当の性能検査とCPUを奪い合わず、
        // 同じ実行条件で10/10の決定性を確かめる。
        let indexed_runs: Vec<(usize, SearchOutcome, f64)> = (0..RUNS)
            .map(|index| {
                let started = std::time::Instant::now();
                let outcome = run_to_completion(&sample);
                (index, outcome, started.elapsed().as_secs_f64())
            })
            .collect();
        for (index, _, elapsed) in &indexed_runs {
            println!(
                "RUN {} {}/{} elapsed={elapsed:.3}s",
                sample.name,
                index + 1,
                RUNS,
            );
        }
        let calculation_seconds: f64 = indexed_runs.iter().map(|(_, _, elapsed)| elapsed).sum();
        let runs: Vec<_> = indexed_runs
            .into_iter()
            .map(|(_, outcome, _)| outcome)
            .collect();
        for (index, run) in runs.iter().enumerate().skip(1) {
            let max_delta = assert_outcome_deterministic(sample.name, run, &runs[0]);
            println!(
                "DETERMINISM {} run={} discrete=exact float_max_delta={max_delta:.3e} tolerance={MEASUREMENT_TOL:.1e}",
                sample.name,
                index + 1
            );
        }

        let session = FoldSession::new(&sample.document)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.name));
        let verified = verify_search_completion(
            &session,
            &runs[0],
            &sample.goal,
            GapWeights::DEFAULT,
            PoseScan::DEFAULT,
            CompletionTolerance::DEFAULT,
        );
        let report = verified.report();
        let ids: Vec<_> = runs[0].steps.iter().map(|step| step.mv.id).collect();
        let reached = CompletionTolerance::DEFAULT.contains(&report.final_gaps);
        println!(
            "TASK26 {} reached={reached} final={:?} ids={ids:?} stop={:?} states={}/{} branch={} {}",
            sample.name,
            report.final_gaps,
            runs[0].stop,
            runs[0].states_expanded,
            runs[0].states_generated,
            runs[0].max_branching,
            report.describe(),
        );

        assert!(
            report.passed(),
            "{}: 返した手順を最後まで折れない: {report:?}",
            sample.name
        );
        assert_eq!(
            report.cleared(),
            report.requested,
            "{}: 手順消化率が100%でない",
            sample.name
        );
        assert_eq!(report.penetrations, 0, "{}: めり込みがある", sample.name);
        assert!(
            report.max_seam_gap < MAX_SEAM_GAP,
            "{}: 裂け{}が上限{MAX_SEAM_GAP}以上",
            sample.name,
            report.max_seam_gap
        );
        assert_eq!(
            report.poses_checked,
            report.requested * PoseScan::DEFAULT.points() + 1,
            "{}: 各手21姿勢と終点を見ていない",
            sample.name
        );
        assert!(
            report.final_check.is_sound(),
            "{}: 終点が不健全",
            sample.name
        );
        for step in &report.steps {
            assert_eq!(step.poses_checked, PoseScan::DEFAULT.points());
            assert_eq!(step.penetrations, 0);
            assert!(step.max_seam_gap < MAX_SEAM_GAP);
        }

        assert!(
            runs[0].states_expanded <= budget.max_states,
            "{}: 状態数上限を超えた",
            sample.name
        );
        assert!(
            runs[0].steps.len() <= budget.max_depth,
            "{}: 深さ上限を超えた",
            sample.name
        );

        // 状態上限の内側で作った状態の数。**実測(2026-08-22、最適化あり)**は
        // 折り鶴12・やっこさん4・鳥の基本形15。実測そのものを境目にせず、
        // 最大15が上限の8割(18.75)に収まる24を境目にする(`CLAUDE.md` §10.7.9)。
        assert!(
            runs[0].states_generated <= 24,
            "{}: 作った状態が{}件へ増えた",
            sample.name,
            runs[0].states_generated
        );

        let mut replay = session.clone();
        let mut proper_subset_moves = 0usize;
        for step in &runs[0].steps {
            // 番号ではなく意味で数える。全網でも方向付きでも層packetでもない、
            // 2本以上の折り線を同時に閉じる手だけが「重なりの部分集合」である。
            if step.mv.id != replay.fold_lines().len()
                && step.mv.closes.len() >= 2
                && !replay.move_is_directional_fold(step.mv.id)
                && !replay.move_uses_layer_packet(step.mv.id)
                && !replay.move_opens_and_closes(step.mv.id)
                && step.mv.id >= replay.fold_lines().len()
            {
                proper_subset_moves += 1;
            }
            replay
                .apply(&step.mv)
                .unwrap_or_else(|error| panic!("{}: 探索手順の再適用: {error}", sample.name));
        }

        match sample.name {
            "折り鶴" => {
                completed += 1;
                typed_states += 1;
                assert!(
                    matches!(&verified, VerifiedPlan::CheckedToFinish(_)),
                    "折り鶴が完成手順型になっていない"
                );
                assert!(reached, "折り鶴が完成しなくなった");
                assert_eq!(runs[0].stop, SearchStop::GoalReached);
                assert!(runs[0].steps.len() >= 2, "折り鶴が1手へ戻った");
                assert!(
                    proper_subset_moves >= 1,
                    "折り鶴が部分集合候補を使っていない"
                );
                assert!(
                    CompletionTolerance::DEFAULT.contains(&report.final_gaps),
                    "折り鶴の4指標が完成許容を外れた: {:?}",
                    report.final_gaps
                );
            }
            "やっこさん" => {
                completed += 1;
                typed_states += 1;
                assert!(
                    matches!(&verified, VerifiedPlan::CheckedToFinish(_)),
                    "やっこさんが完成手順型になっていない"
                );
                assert!(reached, "やっこさんが完成しなくなった");
                assert_eq!(runs[0].stop, SearchStop::GoalReached);
                assert!(
                    report.final_gaps.width < 1e-9,
                    "やっこさんの完成を壊した: {}",
                    report.final_gaps.width
                );
                assert_gaps_near(sample.name, report.final_gaps, FinishGaps::BEST);
            }
            "鳥の基本形" => {
                failed.push(sample.name);
                typed_states += 1;
                let VerifiedPlan::Partial(partial) = &verified else {
                    panic!("鳥の基本形の打ち切り手順が完成手順型になった");
                };
                assert_eq!(partial.stop(), SearchStop::StateCap);
                assert!(!reached, "鳥の基本形の未達項目を隠した");
                assert_eq!(runs[0].stop, SearchStop::StateCap);
                assert!(runs[0].steps.len() >= 2, "鳥の基本形が1手へ戻った");
                assert!(
                    proper_subset_moves >= 1,
                    "鳥の基本形が部分集合候補を使っていない"
                );
                assert!(
                    report.final_gaps.width < BIRD_WIDTH_REGRESSION_LIMIT,
                    "鳥の基本形の太さ改善が後退した: {}",
                    report.final_gaps.width
                );
                assert!(
                    report.final_gaps.length > CompletionTolerance::DEFAULT.length,
                    "鳥の基本形の未達理由を隠した"
                );
            }
            other => panic!("{other}: 未知の標本"),
        }

        println!(
            "SUBSET {} hands={} proper_subsets={} width={:.12} baseline={:.12} upper_ratio={:.2}% wall={:.3}s calculation_sum={calculation_seconds:.3}s average={:.3}s",
            sample.name,
            runs[0].steps.len(),
            proper_subset_moves,
            report.final_gaps.width,
            sample.measured_task_24.width,
            100.0 * report.final_gaps.width / CompletionTolerance::DEFAULT.width,
            sample_started.elapsed().as_secs_f64(),
            calculation_seconds / RUNS as f64,
        );
    }
    assert_eq!(typed_states, 3, "3標本すべてを型で区別していない");
    assert_eq!(completed, 2, "完成許容値を満たした標本数が変わった");
    assert_eq!(failed, ["鳥の基本形"], "未達標本を隠している");
}

/// やっこさんは単線8候補の後ろに、全16折り目を同時に閉じる第9候補を持つ。
/// `branch=3`より後ろのIDでも完成候補を枝刈り前に測り、21姿勢で通す。
#[test]
fn task_26_yakko_uses_a_verified_simultaneous_precrease_move() {
    let sample = yakko_sample();
    let mut session = FoldSession::new(&sample.document).expect("やっこさんを読み込めない");
    assert_eq!(session.crease_lines().len(), 16);
    assert_eq!(session.fold_lines().len(), 8);
    let network_id = session.fold_lines().len();
    let mv = session
        .check_move(network_id, PoseScan::DEFAULT)
        .expect("同時折り候補がない")
        .expect("同時折り候補を21姿勢で折れない");
    assert_eq!(mv.id, 8);
    assert_eq!(mv.closes.len(), 16);
    assert_eq!(mv.poses_checked, 21);
    assert_eq!(mv.penetrations, 0);
    assert!(mv.max_seam_gap < MAX_SEAM_GAP);
    session.apply(&mv).expect("確認した同時折りを進められない");
    let gaps = ori3_propose::finish_gaps(
        &sample.goal.target,
        &sample.goal.measure(session.document()),
    );
    assert!(CompletionTolerance::DEFAULT.contains(&gaps));
    assert_gaps_near(sample.name, gaps, FinishGaps::BEST);
}

/// 鶴と鳥の基本形は初期全網を閉じられないが、1手後には層の上下で重なった
/// proper subset が現れ、21姿勢を通る。全部分集合を列挙しない絞り込みを固定する。
#[test]
fn a_safe_coincident_partial_network_appears_after_the_first_fold() {
    let tolerance = CompletionTolerance::DEFAULT;
    for (sample, first_id) in [(crane_sample(), 16), (bird_base_sample(), 2)] {
        let mut session = FoldSession::new(&sample.document)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.name));
        let network_id = session.fold_lines().len();
        // 候補は「全網」「重なりの部分集合」「方向付き単線」「層packet技法」の4種で、
        // 番号だけでは種類が分からない。ここで数えたいのは**重なりの部分集合**、
        // すなわち全網でも方向付きでも層packetでもない、2本以上を同時に閉じる手である。
        let coincident_subsets = |session: &FoldSession| -> Vec<_> {
            session
                .verified_network_moves(PoseScan::DEFAULT)
                .into_iter()
                .filter(|mv| {
                    mv.id != network_id
                        && mv.closes.len() >= 2
                        && !session.move_is_directional_fold(mv.id)
                        && !session.move_uses_layer_packet(mv.id)
                        && !session.move_opens_and_closes(mv.id)
                })
                .collect()
        };
        assert!(
            coincident_subsets(&session).is_empty(),
            "{}: 折る前から重なりの部分集合が現れた",
            sample.name
        );
        assert!(matches!(
            session.check_move(network_id, PoseScan::DEFAULT),
            Some(Err(Unverified::CannotCollapse))
        ));
        let first = session
            .verify_move(first_id, PoseScan::DEFAULT)
            .unwrap_or_else(|| panic!("{}: 初手{first_id}を折れない", sample.name));
        session
            .apply(&first)
            .unwrap_or_else(|error| panic!("{}: 初手を進められない: {error}", sample.name));
        let partial = coincident_subsets(&session)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{}: 21姿勢を通るproper subsetがない", sample.name));
        assert!(
            partial.closes.len() < session.crease_lines().len(),
            "{}: 全網を部分集合として数えた",
            sample.name
        );
        assert_eq!(partial.poses_checked, 21);
        assert_eq!(partial.penetrations, 0);
        assert!(partial.max_seam_gap < MAX_SEAM_GAP);

        let gaps = sample.measured_task_24;
        println!(
            "TASK26-FAIL {} lines={} length={:.6}/{:.6} ({:.1}%) width={:.6}/{:.6} ({:.1}%) position={:.6}/{:.6} ({:.1}%)",
            sample.name,
            network_id,
            gaps.length,
            tolerance.length,
            100.0 * gaps.length / tolerance.length,
            gaps.width,
            tolerance.width,
            100.0 * gaps.width / tolerance.width,
            gaps.position,
            tolerance.position,
            100.0 * gaps.position / tolerance.position,
        );
        assert!(
            gaps.length > tolerance.length
                || gaps.width > tolerance.width
                || gaps.position > tolerance.position
        );
    }
}
