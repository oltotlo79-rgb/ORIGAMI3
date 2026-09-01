//! 作業24〜26: 実作品の独立した完成形に対する終点測定と完成探索。
//!
//! 標本は折り鶴・やっこさん・鳥の基本形の3件。完成目標は探索結果から作らず、
//! 各作品の既存受け入れ手順を最後まで折った参照形から先に固定している。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ori3_model::{CreasePattern, Document, EdgeKind, Paper};
use ori3_propose::enumerate::{FoldSession, MAX_SEAM_GAP, PoseScan, Unverified};
use ori3_propose::finish::{FinishGaps, FinishTarget, TargetTip};
use ori3_propose::search::{
    CompletionTolerance, FoldGoal, GapWeights, SearchBudget, SearchOutcome, SearchStop, TipSite,
    search_to_completion, search_to_finish,
};
use ori3_propose::skeleton::TipPos2d;
use ori3_propose::verify::{VerifiedPlan, verify_search_completion, verify_search_outcome};
use serde::{Deserialize, Serialize};

struct Sample {
    id: &'static str,
    name: &'static str,
    document: Document,
    goal: FoldGoal,
}

const RUNS: usize = 10;
const TASK24_RECORD_SCHEMA_VERSION: u32 = 1;
const TASK24_MEASUREMENT_BASIS: &str = "search_to_finish + fresh FoldSession replay + verify_search_outcome at 21 poses per step + pre-step CP layer-order validation";

/// 部分集合候補を加えた実測から、改善量のおよそ80%を恒久に守る境目。
///
/// 実測値そのもの（折り鶴0.172464、鳥の基本形は7e-16未満）を境目にはせず、
/// 変更前との差の約20%を回帰余裕として残す。
const BIRD_WIDTH_REGRESSION_LIMIT: f64 = 0.34;

/// 記録した作業24のgapとの絶対照合差。
///
/// JSON往復の実測差は`0.0`、完成参照との数値残差は最大`1.77e-15`。完成/未完成を分ける
/// 最小差は、やっこさんの太さ `0.3106601718 - 0.2485281374 = 0.0621320344`。
/// `1e-9` は揺れより5桁以上粗く、形の差より7桁以上細い。作業25の完成許容値
/// ではなく、計算した小数を記録と厳密一致させないためだけの照合差である。
const TASK24_GAP_ABS_TOLERANCE: f64 = 1e-9;

/// 作業24で折り線・保存driverの座標を照合する絶対差。
///
/// CIで観測した座標ずれは最大`1.11e-16`であり、`1e-12`は約9,000倍の余裕を持つ。
/// 一方、別頂点間の実測最短距離`1.29e-3`より9桁細く、別の点を同一視しない。
const TASK24_COORDINATE_ABS_TOLERANCE: f64 = 1e-12;

/// 作業24で保存driverの平坦終点角を照合する絶対差（度）。
///
/// 10回の反復で観測した角度差は`0°`。`PoseScan::DEFAULT`の隣接姿勢距離は
/// `180° / 20 = 9°`なので、`1e-9°`は別姿勢を混同しない。
const TASK24_ANGLE_ABS_TOLERANCE_DEGREES: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task24CompletionRecord {
    schema_version: u32,
    measurement_basis: String,
    samples: Vec<RecordedTask24Sample>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedTask24Sample {
    sample_id: String,
    gaps: RecordedFinishGaps,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedFinishGaps {
    count: f64,
    length: f64,
    width: f64,
    position: f64,
}

impl RecordedFinishGaps {
    fn as_finish_gaps(self) -> FinishGaps {
        FinishGaps {
            count: self.count,
            length: self.length,
            width: self.width,
            position: self.position,
        }
    }
}

impl From<FinishGaps> for RecordedFinishGaps {
    fn from(gaps: FinishGaps) -> Self {
        Self {
            count: gaps.count,
            length: gaps.length,
            width: gaps.width,
            position: gaps.position,
        }
    }
}

fn task_24_completion_record_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus/task-24-completion-gaps.json")
}

fn assert_task_24_record_valid(record: &Task24CompletionRecord) {
    assert_eq!(
        record.schema_version, TASK24_RECORD_SCHEMA_VERSION,
        "task 24記録のschema versionが違う"
    );
    assert_eq!(
        record.measurement_basis, TASK24_MEASUREMENT_BASIS,
        "task 24記録の測定根拠が違う"
    );
    let sample_ids = record
        .samples
        .iter()
        .map(|sample| sample.sample_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        sample_ids,
        ["crane", "yakko", "bird-base"],
        "task 24記録の標本IDまたは順序が違う"
    );
    for sample in &record.samples {
        for (field, value) in [
            ("count", sample.gaps.count),
            ("length", sample.gaps.length),
            ("width", sample.gaps.width),
            ("position", sample.gaps.position),
        ] {
            assert!(
                value.is_finite() && value >= 0.0,
                "{}の{field}が有限な非負値でない: {value}",
                sample.sample_id
            );
        }
    }
}

fn read_task_24_completion_record() -> Task24CompletionRecord {
    let path = task_24_completion_record_path();
    let bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("{}を読めない: {error}", path.display()));
    let record: Task24CompletionRecord = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("{}のschemaが不正: {error}", path.display()));
    assert_task_24_record_valid(&record);
    record
}

fn recorded_task_24_gaps(record: &Task24CompletionRecord, sample_id: &str) -> FinishGaps {
    record
        .samples
        .iter()
        .find(|sample| sample.sample_id == sample_id)
        .unwrap_or_else(|| panic!("task 24記録に標本がない: {sample_id}"))
        .gaps
        .as_finish_gaps()
}

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
        id: "crane",
        name: "折り鶴",
        document: fixture_document("cp-crane.json"),
        goal: FoldGoal {
            target,
            body: [0.5, 0.5],
            sites: corner_sites(),
            layer_target: None,
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
        id: "yakko",
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
            layer_target: None,
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
        id: "bird-base",
        name: "鳥の基本形",
        document: fixture_document("cp-bird-base.json"),
        goal: FoldGoal {
            target,
            body: [0.5, 0.5],
            sites: corner_sites(),
            layer_target: None,
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
            delta <= TASK24_GAP_ABS_TOLERANCE,
            "{name}: {measure}の実測{got:.12}が記録{want:.12}から{TASK24_GAP_ABS_TOLERANCE:.1e}より大きく動いた"
        );
        max_delta = max_delta.max(delta);
    }
    max_delta
}

fn assert_task_24_record_near(name: &str, got: FinishGaps, want: FinishGaps) -> f64 {
    assert_gaps_near(name, got, want)
}

fn assert_scalar_near(
    name: &str,
    field: &str,
    got: f64,
    want: f64,
    tolerance: f64,
) -> f64 {
    let delta = (got - want).abs();
    assert!(
        delta <= tolerance,
        "{name}: {field}の{RUNS}回比較差|{got} - {want}|が{tolerance:.1e}を超えた"
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
        TASK24_GAP_ABS_TOLERANCE,
    ));
    max_delta = max_delta.max(assert_scalar_near(
        name,
        "終点点数",
        got.best_score,
        want.best_score,
        TASK24_GAP_ABS_TOLERANCE,
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
                    TASK24_COORDINATE_ABS_TOLERANCE,
                ));
            }
        }
        max_delta = max_delta.max(assert_scalar_near(
            name,
            &format!("{step}の裂け"),
            got.mv.max_seam_gap,
            want.mv.max_seam_gap,
            TASK24_GAP_ABS_TOLERANCE,
        ));
        max_delta = max_delta.max(assert_gaps_near(name, got.gaps, want.gaps));
        max_delta = max_delta.max(assert_scalar_near(
            name,
            &format!("{step}の点数"),
            got.score,
            want.score,
            TASK24_GAP_ABS_TOLERANCE,
        ));
    }
    max_delta
}

#[derive(Debug)]
struct Task24LayerOrderAudit {
    audited_orders: usize,
    checked: usize,
    violations: usize,
    discarded: usize,
}

#[derive(Debug)]
struct Task24Measurement {
    gaps: FinishGaps,
    audit: Task24LayerOrderAudit,
}

/// 探索が採用した全手の保存順を、候補順から独立した物理制約で監査する。
fn audit_task_24_adopted_layer_orders(
    sample: &Sample,
    outcome: &SearchOutcome,
    run: usize,
    total_runs: usize,
) -> Task24LayerOrderAudit {
    let mut folded = FoldSession::new(&sample.document)
        .unwrap_or_else(|error| panic!("{}: {error}", sample.name));
    let mut audit = Task24LayerOrderAudit {
        audited_orders: 0,
        checked: 0,
        violations: 0,
        discarded: 0,
    };

    for (index, ranked) in outcome.steps.iter().enumerate() {
        // 必ずapply前の入力CPを保存する。apply後のCPを渡すと
        // `settle_kinds_from_order`後のM/Vが候補順を自己認証し、旧候補16で実測した
        // 一般制約2/37違反・物理的な`discarded_relations` 4組を見逃す。
        // 表示用total化の失敗1組は`display_resolution_failure`へ別記録される。
        let input_cp = folded.document().cp.clone();
        let Some(Ok(checked)) = folded.check_move(ranked.mv.id, PoseScan::DEFAULT) else {
            panic!("{}: {}手目を再検証できない", sample.name, index + 1);
        };
        folded.apply(&checked).unwrap_or_else(|error| {
            panic!(
                "{}: {}手目を再適用できない: {error}",
                sample.name,
                index + 1
            )
        });

        let document = folded.document();
        let faces = ori3_cp::extract_faces(&document.cp);
        let up_to = document.sequence.len();
        let (state, warnings) = ori3_layers::flat_state_at(document, &faces, up_to)
            .unwrap_or_else(|error| {
                panic!("{}: {}手目の平坦状態を再生できない: {error}", sample.name, index + 1)
            });
        assert!(
            warnings.is_empty(),
            "{}: {}手目の平坦状態に警告がある: {warnings:?}",
            sample.name,
            index + 1
        );
        let saved_order = ori3_layers::saved_layer_order_at(document, &faces, up_to, 1.0)
            .unwrap_or_else(|| {
                panic!("{}: {}手目に有効な保存層順がない", sample.name, index + 1)
            });
        assert_eq!(
            saved_order, state.order,
            "{}: {}手目の保存順と平坦状態の順が違う",
            sample.name, index + 1
        );
        let expected_faces = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
        let saved_faces = saved_order.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            saved_order.len(), faces.len(),
            "{}: {}手目の保存順が完全permutationでない",
            sample.name, index + 1
        );
        assert_eq!(
            saved_faces, expected_faces,
            "{}: {}手目の保存順に面の過不足がある",
            sample.name, index + 1
        );

        let validation = ori3_layers::precrease_collapse::validate_precrease_layer_order(
            &input_cp,
            &faces,
            &state.placements,
            &saved_order,
        )
        .unwrap_or_else(|error| {
            panic!("{}: {}手目の一般制約を導けない: {error}", sample.name, index + 1)
        });
        let checked = validation.counts.adjacent_folds
            + validation.counts.taco_tortilla
            + validation.counts.taco_taco
            + validation.counts.continuous;
        let violations = validation.violations.adjacent_folds.len()
            + validation.violations.taco_tortilla.len()
            + validation.violations.taco_taco.len()
            + validation.violations.continuous_crossings.len()
            + validation.violations.continuous.len();
        assert!(
            validation.violations.duplicate_faces.is_empty(),
            "{}: {}手目の保存順に重複面がある: {:?}",
            sample.name,
            index + 1,
            validation.violations.duplicate_faces
        );
        assert!(
            validation.violations.missing_faces.is_empty(),
            "{}: {}手目の保存順に欠落面がある: {:?}",
            sample.name,
            index + 1,
            validation.violations.missing_faces
        );
        assert!(
            validation.violations.unexpected_faces.is_empty(),
            "{}: {}手目の保存順に未知面がある: {:?}",
            sample.name,
            index + 1,
            validation.violations.unexpected_faces
        );
        assert_eq!(
            violations, 0,
            "{}: {}手目に一般制約違反がある: {:?}",
            sample.name, index + 1, validation.violations
        );
        assert!(
            validation.discarded_relations.is_empty(),
            "{}: {}手目に破棄された関係がある: {:?}",
            sample.name,
            index + 1,
            validation.discarded_relations
        );
        assert!(
            validation.is_valid(),
            "{}: {}手目の保存順が無効: {validation:?}",
            sample.name,
            index + 1
        );

        let stored_step = document.sequence.last().unwrap_or_else(|| {
            panic!("{}: {}手目がDocumentへ保存されていない", sample.name, index + 1)
        });
        assert!(
            !stored_step.drivers.is_empty(),
            "{}: {}手目に保存driverがない",
            sample.name,
            index + 1
        );
        for driver in &stored_step.drivers {
            let flat_delta = (driver.target_angle_deg.abs() - 180.0).abs();
            assert!(
                flat_delta <= TASK24_ANGLE_ABS_TOLERANCE_DEGREES,
                "{}: {}手目の保存角{}°が平坦終点から{}°ずれた",
                sample.name,
                index + 1,
                driver.target_angle_deg,
                flat_delta
            );
        }

        audit.audited_orders += 1;
        audit.checked += checked;
        audit.violations += violations;
        audit.discarded += validation.discarded_relations.len();
        println!(
            "TASK24_LAYER sample={} run={run}/{total_runs} step={}/{} violations={violations}/{checked} discarded={}",
            sample.name,
            index + 1,
            outcome.steps.len(),
            validation.discarded_relations.len()
        );
    }

    assert!(
        audit.audited_orders >= 1,
        "{}: 採用層順を1件も監査していない",
        sample.name
    );
    audit
}

fn measure_task_24_sample(sample: &Sample, run: usize, total_runs: usize) -> Task24Measurement {
    let session =
        FoldSession::new(&sample.document).unwrap_or_else(|error| panic!("{}: {error}", sample.name));
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
    assert!(
        report.passed(),
        "{}: 探索手順を最後まで安全に折れない: {report:?}",
        sample.name
    );
    let audit = audit_task_24_adopted_layer_orders(sample, &outcome, run, total_runs);
    let ids = outcome
        .steps
        .iter()
        .map(|step| step.mv.id)
        .collect::<Vec<_>>();
    println!(
        "TASK24 {} run={run}/{total_runs} start={:?} final={:?} position_bits=0x{:016x} ids={ids:?} stop={:?} states={}/{} branch={} elapsed={:.3}s audit={}/{} discarded={} {}",
        sample.name,
        report.start_gaps,
        report.final_gaps,
        report.final_gaps.position.to_bits(),
        outcome.stop,
        outcome.states_expanded,
        outcome.states_generated,
        outcome.max_branching,
        started.elapsed().as_secs_f64(),
        audit.violations,
        audit.checked,
        audit.discarded,
        report.describe(),
    );
    Task24Measurement {
        gaps: report.final_gaps,
        audit,
    }
}

/// 作業24: 固定記録をread-onlyで読み、許容値を使わない正規再測定と照合する。
#[test]
fn task_24_measures_actual_completion_gaps_before_setting_tolerances() {
    let record = read_task_24_completion_record();
    for sample in samples() {
        let measurement = measure_task_24_sample(&sample, 1, 1);
        let recorded = recorded_task_24_gaps(&record, sample.id);
        assert_task_24_record_near(sample.name, measurement.gaps, recorded);
    }
}

fn median_task_24_component(sample: &str, field: &str, mut values: Vec<f64>) -> f64 {
    assert_eq!(values.len(), RUNS, "{sample}: {field}の測定回数が違う");
    values.sort_by(f64::total_cmp);
    let spread = values[RUNS - 1] - values[0];
    assert!(
        spread <= TASK24_GAP_ABS_TOLERANCE,
        "{sample}: {field}の{RUNS}回差{spread}が{TASK24_GAP_ABS_TOLERANCE}を超えた"
    );
    values[RUNS / 2]
}

fn median_task_24_gaps(sample: &str, measurements: &[Task24Measurement]) -> FinishGaps {
    FinishGaps {
        count: median_task_24_component(
            sample,
            "count",
            measurements.iter().map(|run| run.gaps.count).collect(),
        ),
        length: median_task_24_component(
            sample,
            "length",
            measurements.iter().map(|run| run.gaps.length).collect(),
        ),
        width: median_task_24_component(
            sample,
            "width",
            measurements.iter().map(|run| run.gaps.width).collect(),
        ),
        position: median_task_24_component(
            sample,
            "position",
            measurements
                .iter()
                .map(|run| run.gaps.position)
                .collect(),
        ),
    }
}

fn remove_task_24_temporary_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}を消せない: {error}", path.display())),
    }
}

fn stage_task_24_completion_record(path: &Path, candidate_bytes: &[u8]) -> PathBuf {
    let candidate: Task24CompletionRecord = serde_json::from_slice(candidate_bytes)
        .unwrap_or_else(|error| panic!("task 24候補記録のschemaが不正: {error}"));
    assert_task_24_record_valid(&candidate);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("task 24記録のfile name");
    let staged_path = path.with_file_name(format!(
        ".{file_name}.{}.{}.staged",
        std::process::id(),
        nonce
    ));
    let staged = (|| -> Result<(), String> {
        fs::write(&staged_path, candidate_bytes)
            .map_err(|error| format!("{}を書けない: {error}", staged_path.display()))?;
        let readback = fs::read(&staged_path)
            .map_err(|error| format!("{}を再読込できない: {error}", staged_path.display()))?;
        if readback != candidate_bytes {
            return Err(format!("{}のreadbackが不一致", staged_path.display()));
        }
        let staged_record: Task24CompletionRecord = serde_json::from_slice(&readback)
            .map_err(|error| format!("staged task 24 record schema: {error}"))?;
        assert_task_24_record_valid(&staged_record);
        Ok(())
    })();
    if let Err(error) = staged {
        let cleanup = remove_task_24_temporary_file(&staged_path);
        panic!("task 24記録の同directory preflight失敗: {error}; cleanup={cleanup:?}");
    }
    staged_path
}

fn restore_task_24_original_record(
    path: &Path,
    staged_path: &Path,
    backup_path: &Path,
    original_bytes: &[u8],
) -> Result<(), String> {
    if backup_path.exists() {
        remove_task_24_temporary_file(path)?;
        fs::rename(backup_path, path).map_err(|error| {
            format!(
                "{}から{}へ元記録を戻せない: {error}",
                backup_path.display(),
                path.display()
            )
        })?;
    }
    remove_task_24_temporary_file(staged_path)?;
    let restored = fs::read(path)
        .map_err(|error| format!("{}をrollback後に読めない: {error}", path.display()))?;
    if restored != original_bytes {
        return Err("rollback後のtask 24記録が元bytesと違う".to_owned());
    }
    Ok(())
}

/// 正本を書けるのは、下の`#[ignore]`再生成testからこの関数を呼ぶ経路だけ。
fn write_task_24_completion_record(
    record: &Task24CompletionRecord,
    original_bytes: &[u8],
) -> usize {
    assert_task_24_record_valid(record);
    let mut candidate_bytes =
        serde_json::to_vec_pretty(record).expect("task 24記録をJSONへ変換できない");
    candidate_bytes.push(b'\n');
    let path = task_24_completion_record_path();
    let staged_path = stage_task_24_completion_record(&path, &candidate_bytes);
    let backup_path = staged_path.with_extension("backup");
    assert!(
        !backup_path.exists(),
        "task 24 backupが既にある: {}",
        backup_path.display()
    );

    let replacement = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // 測定中に別担当が正本を変えていたら上書きしない。
        assert_eq!(
            fs::read(&path)
                .unwrap_or_else(|error| panic!("{}をwrite直前に読めない: {error}", path.display()))
                .as_slice(),
            original_bytes,
            "task 24記録が測定中に変わった"
        );
        fs::rename(&path, &backup_path).unwrap_or_else(|error| {
            panic!(
                "{}を{}へ退避できない: {error}",
                path.display(),
                backup_path.display()
            )
        });
        fs::rename(&staged_path, &path).unwrap_or_else(|error| {
            panic!(
                "{}を{}へ置換できない: {error}",
                staged_path.display(),
                path.display()
            )
        });
        let readback = fs::read(&path)
            .unwrap_or_else(|error| panic!("{}を更新後に読めない: {error}", path.display()));
        assert_eq!(readback, candidate_bytes, "task 24更新記録のreadback不一致");
        let updated: Task24CompletionRecord = serde_json::from_slice(&readback)
            .unwrap_or_else(|error| panic!("task 24更新記録のschemaが不正: {error}"));
        assert_task_24_record_valid(&updated);
        assert_task_24_records_near("task 24更新記録", &updated, record);
        fs::remove_file(&backup_path).unwrap_or_else(|error| {
            panic!("{}を更新後に消せない: {error}", backup_path.display())
        });
    }));
    if let Err(payload) = replacement {
        restore_task_24_original_record(
            &path,
            &staged_path,
            &backup_path,
            original_bytes,
        )
        .unwrap_or_else(|error| panic!("task 24記録のrollback失敗: {error}"));
        std::panic::resume_unwind(payload);
    }
    candidate_bytes.len()
}

#[test]
#[ignore = "task 24 completion-gap recordの明示的な再生成専用"]
fn regenerate_task_24_completion_gap_record() {
    assert_eq!(
        std::env::var("ORI3_REGENERATE_TASK24_RECORD").as_deref(),
        Ok("1"),
        "ORI3_REGENERATE_TASK24_RECORD=1を明示した場合だけ再生成できる"
    );

    let path = task_24_completion_record_path();
    let original_bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("{}の元bytesを読めない: {error}", path.display()));
    let mut recorded_samples = Vec::new();
    for sample in samples() {
        let measurements = (1..=RUNS)
            .map(|run| measure_task_24_sample(&sample, run, RUNS))
            .collect::<Vec<_>>();
        let position_bits = measurements[0].gaps.position.to_bits();
        let position_bit_matches = measurements
            .iter()
            .filter(|measurement| measurement.gaps.position.to_bits() == position_bits)
            .count();
        let gaps = median_task_24_gaps(sample.name, &measurements);
        let audit_checked = measurements
            .iter()
            .map(|measurement| measurement.audit.checked)
            .collect::<Vec<_>>();
        assert!(
            measurements.iter().all(|measurement| {
                measurement.audit.violations == 0 && measurement.audit.discarded == 0
            }),
            "{}: 10回監査に違反または破棄がある",
            sample.name
        );
        println!(
            "TASK24_REGENERATE sample={} position={} bits=0x{position_bits:016x} bit_matches={position_bit_matches}/{RUNS} audit_checked={audit_checked:?} violations=0 discarded=0",
            sample.name,
            gaps.position
        );
        recorded_samples.push(RecordedTask24Sample {
            sample_id: sample.id.to_owned(),
            gaps: gaps.into(),
        });
    }
    let record = Task24CompletionRecord {
        schema_version: TASK24_RECORD_SCHEMA_VERSION,
        measurement_basis: TASK24_MEASUREMENT_BASIS.to_owned(),
        samples: recorded_samples,
    };
    let bytes = write_task_24_completion_record(&record, &original_bytes);
    let record_delta = assert_task_24_records_near(
        "再生成したtask 24記録と通常reader",
        &read_task_24_completion_record(),
        &record,
    );
    println!("WROTE {} bytes={bytes} semantic_max_delta={record_delta:.3e} tolerance={TASK24_GAP_ABS_TOLERANCE:.1e}", path.display());
}

/// 作業25の4許容を従来値へ完全に凍結し、正規再測定の80%以下であることを守る。
///
/// 記録は2026-08-28の正規再測定値（一般制約違反0・破棄0・10回連続bit一致）。
/// 許容は同日の利用者決定で従来値を据え置いた。従来の決め方（実測の8割）より
/// 厳しくなっても、基準を緩めないための決定である。新記録に対する実測比率は
/// length `80%`、width `60.000000000000085%`、position `74.13562556544%`。
/// countの3記録は`0.0`なので除算せず、離散的な最小非0値`1/12`に対する`80%`を守る。
#[test]
fn task_25_freezes_every_tolerance_without_exceeding_eighty_percent_bases() {
    let record = read_task_24_completion_record();
    let tolerance = CompletionTolerance::DEFAULT;

    const FROZEN_COUNT_TOLERANCE: f64 = 0.066_666_666_666_666_67;
    const FROZEN_LENGTH_TOLERANCE: f64 = 0.565_685_424_949_238_7;
    const FROZEN_WIDTH_TOLERANCE: f64 = 0.248_528_137_423_857_1;
    const FROZEN_POSITION_TOLERANCE: f64 = 0.346_118_308_825_409_8;
    assert_eq!(
        tolerance.count, FROZEN_COUNT_TOLERANCE,
        "count許容が凍結値から動いた"
    );
    assert_eq!(
        tolerance.length, FROZEN_LENGTH_TOLERANCE,
        "length許容が凍結値から動いた"
    );
    assert_eq!(
        tolerance.width, FROZEN_WIDTH_TOLERANCE,
        "width許容が凍結値から動いた"
    );
    assert_eq!(
        tolerance.position, FROZEN_POSITION_TOLERANCE,
        "position許容が凍結値から動いた"
    );

    let length_record = recorded_task_24_gaps(&record, "bird-base").length;
    let width_record = recorded_task_24_gaps(&record, "yakko").width;
    let position_record = recorded_task_24_gaps(&record, "crane").position;
    for (measure, limit, actual) in [
        ("長さ", tolerance.length, length_record),
        ("太さ", tolerance.width, width_record),
        ("位置", tolerance.position, position_record),
    ] {
        let eighty_percent = 0.8 * actual;
        assert!(
            limit <= eighty_percent,
            "{measure}: 据え置き許容{limit}が正規記録{actual}の80%={eighty_percent}を超えた"
        );
    }

    // countの記録は3作品とも0なので、記録値で割る比率は定義できない。
    // 代わりに許容が非負で、最大12葉の1本欠けに相当する最小非0値1/12の80%以下かを守る。
    const MIN_NONZERO_COUNT_GAP: f64 = 1.0 / 12.0;
    assert!(tolerance.count >= 0.0, "count許容が負になった");
    assert!(
        tolerance.count <= 0.8 * MIN_NONZERO_COUNT_GAP,
        "count許容{}が離散根拠1/12の80%を超えた",
        tolerance.count
    );

    let length_ratio = tolerance.length / length_record;
    let width_ratio = tolerance.width / width_record;
    let position_ratio = tolerance.position / position_record;
    println!(
        "TASK25_RATIOS count_record=0 count_discrete_ratio={:.15}% length={:.15}% width={:.15}% position={:.15}%",
        100.0 * tolerance.count / MIN_NONZERO_COUNT_GAP,
        100.0 * length_ratio,
        100.0 * width_ratio,
        100.0 * position_ratio,
    );

    assert!(!tolerance.contains(&FinishGaps {
        count: 1.0 / 12.0,
        length: 0.0,
        width: 0.0,
        position: 0.0,
    }));
    for sample in samples() {
        let measured = recorded_task_24_gaps(&record, sample.id);
        assert!(
            !tolerance.contains(&measured),
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
    let task_24_record = read_task_24_completion_record();
    let mut completed = 0usize;
    let mut typed_states = 0usize;
    let budget = SearchBudget::DEFAULT;
    assert_eq!(budget.max_states, 12, "状態数上限を緩めた");
    assert_eq!(budget.branch, 3, "保持する分岐数上限を緩めた");
    assert_eq!(budget.max_depth, 8, "深さ上限を緩めた");
    for sample in samples() {
        let measured_task_24 = recorded_task_24_gaps(&task_24_record, sample.id);
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
                "DETERMINISM {} run={} discrete=exact float_max_delta={max_delta:.3e} tolerance={TASK24_GAP_ABS_TOLERANCE:.1e}",
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

        // 状態上限の内側で作った状態の数。**実測(2026-08-23、最適化あり、
        // `enumerate.rs::WITH_EXTRA_CANDIDATES = true`)**は
        // 折り鶴31・やっこさん4・鳥の基本形16。
        // これは**探索の打ち切りではなく、作った状態の数が増えていないかを見る記録値**
        // である(打ち切りは `max_states`・`branch`・`max_depth` で、そちらは
        // 12・3・8 のまま**1つも上げていない**。この検査の冒頭で確かめている)。
        //
        // 前は24だった。方向付き単線・つぶし折り・花弁折りの候補を作るように
        // したので、1状態から作る子の数が増えた(実測: 折り鶴12→31)。
        // 実測そのものを境目にせず、最大31が上限の8割(38.4)に収まる48を境目にする
        // (`CLAUDE.md` §10.7.9。前の値も 実測15 → 上限24 と同じ取り方だった)。
        assert!(
            runs[0].states_generated <= 48,
            "{}: 作った状態が{}件へ増えた",
            sample.name,
            runs[0].states_generated
        );

        let mut replay = session.clone();
        let mut proper_subset_moves = 0usize;
        // 層を持ち替える手(つぶし折り・花弁折り・開いて閉じる手)の数。
        // 鳥の基本形は、この種の手が無いと長さの隔たりが動かない。
        let mut layer_packet_moves = 0usize;
        for step in &runs[0].steps {
            if replay.move_uses_layer_packet(step.mv.id) || replay.move_opens_and_closes(step.mv.id)
            {
                layer_packet_moves += 1;
            }
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
            let Some(Ok(checked)) = replay.check_move(step.mv.id, PoseScan::DEFAULT) else {
                panic!("{}: 探索手順を再検証できない", sample.name);
            };
            replay
                .apply(&checked)
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
            // 2026-08-23に**完成するようになった**。緩めたのではなく、
            // 直った事実へ書き直したものである(`CLAUDE.md` §5)。
            //
            // それまでは長さの隔たりが `0.7071067811865483` から動かなかった。
            // 理由は2つの取り違えで、どちらも実測で特定して直した
            // (`scratchpad/petal-tear-cause-report.md`)。
            //
            // 1. `enumerate.rs::PART_LAYER_SKIP_MARK` — `flat_motion` が
            //    **動きの部品ごと**に出す「その部品に掛からない層を外した」知らせを、
            //    「折り上がりが指定と違う」と誤読して候補を捨てていた。
            //    鳥を完成させる花弁折りが、これで消えていた。
            // 2. `search.rs::PREPARATION_TURN` — 花弁折りでできた状態が
            //    「準備手の状態」として常に後回しにされ、状態上限12に達するまで
            //    一度も広げられなかった(粗い順位では1位に付けていた)。
            //
            // 実測(最適化あり、10回): `GoalReached` 5手 `[2, 13, 7, 154, 13]`、
            // 数 `0.000000` / 長さ `0.3535533905932740` / 太さ `0.000000000000` /
            // 位置 `0.125000`。決定性は10/10で、最適化なしでも同じ手順・同じ長さになる。
            "鳥の基本形" => {
                completed += 1;
                typed_states += 1;
                assert!(
                    matches!(&verified, VerifiedPlan::CheckedToFinish(_)),
                    "鳥の基本形が完成手順型になっていない"
                );
                assert!(reached, "鳥の基本形が完成しなくなった");
                assert_eq!(runs[0].stop, SearchStop::GoalReached);
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
                    CompletionTolerance::DEFAULT.contains(&report.final_gaps),
                    "鳥の基本形の4指標が完成許容を外れた: {:?}",
                    report.final_gaps
                );
                // 花弁折り(袋を開いて折り返す手)を実際に使っていること。
                // これを使わない限り、長さの隔たりは `0.7071067811865483` から動かない。
                assert!(
                    layer_packet_moves >= 1,
                    "鳥の基本形が層を持ち替える手を使っていない"
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
            measured_task_24.width,
            100.0 * report.final_gaps.width / CompletionTolerance::DEFAULT.width,
            sample_started.elapsed().as_secs_f64(),
            calculation_seconds / RUNS as f64,
        );
    }
    assert_eq!(typed_states, 3, "3標本すべてを型で区別していない");
    // 2026-08-23に鳥の基本形が完成するようになり、**3標本すべてが完成**になった。
    // どれか1つでも完成しなくなれば、その標本の枝の主張が先に落ちる。
    assert_eq!(completed, 3, "完成許容値を満たした標本数が変わった");
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
    let movement = mv.movement();
    assert_eq!(movement.id, 8);
    assert_eq!(movement.closes.len(), 16);
    assert_eq!(movement.poses_checked, 21);
    assert_eq!(movement.penetrations, 0);
    assert!(movement.max_seam_gap < MAX_SEAM_GAP);
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
    let task_24_record = read_task_24_completion_record();
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
                    let movement = mv.movement();
                    movement.id != network_id
                        && movement.closes.len() >= 2
                        && !session.move_is_directional_fold(movement.id)
                        && !session.move_uses_layer_packet(movement.id)
                        && !session.move_opens_and_closes(movement.id)
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
        let movement = partial.movement();
        assert!(
            movement.closes.len() < session.crease_lines().len(),
            "{}: 全網を部分集合として数えた",
            sample.name
        );
        assert_eq!(movement.poses_checked, 21);
        assert_eq!(movement.penetrations, 0);
        assert!(movement.max_seam_gap < MAX_SEAM_GAP);

        let gaps = recorded_task_24_gaps(&task_24_record, sample.id);
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

/// 画像確認用の作品ファイルを書き出す(一時的。確認が終わったら削除する)。
///
/// `verification/check-crane.ori3` / `check-bird-base.ori3` / `check-yakko.ori3` を、
/// **探索が実際に返した手順をそのまま `FoldSession::apply` で進めた結果**から作る。
/// 手で組み立て直してはいない。`Document` をまるごと書き出すので
/// `display` も入る(製品の読み取り機 `store.rs::parse_document` が要求する項目)。
#[test]
#[ignore]
fn zz_write_check_documents() {
    for (sample, name) in [
        (crane_sample(), "check-crane"),
        (bird_base_sample(), "check-bird-base"),
        (yakko_sample(), "check-yakko"),
    ] {
        let outcome = run_to_completion(&sample);
        let mut session = FoldSession::new(&sample.document)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.name));
        for step in &outcome.steps {
            let Some(Ok(checked)) = session.check_move(step.mv.id, PoseScan::DEFAULT) else {
                panic!("{}: 手順を再検証できない", sample.name);
            };
            session
                .apply(&checked)
                .unwrap_or_else(|error| panic!("{}: 手順の再適用: {error}", sample.name));
        }
        let ids: Vec<_> = outcome.steps.iter().map(|step| step.mv.id).collect();
        let path = format!(
            "{}/../../verification/{name}.ori3",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = serde_json::to_string_pretty(session.document()).expect("作品を書き出せない");
        std::fs::write(&path, &text).unwrap_or_else(|error| panic!("{path}: {error}"));

        // 製品の読み取り機と同じ道で読み直す
        // (`apps/desktop/src-tauri/src/store.rs::parse_document` は、
        //  `schema_version` を確かめてから `SavedDocument` へ読み込む)。
        let saved = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let value: serde_json::Value =
            serde_json::from_str(&saved).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert_eq!(
            value.get("schema_version").and_then(serde_json::Value::as_u64),
            Some(u64::from(ori3_model::SCHEMA_VERSION)),
            "{path}: 製品の読み取り機が受け付ける版ではない"
        );
        let reread: ori3_model::SavedDocument = serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("{path}: 製品の読み取り機で読めない: {error}"));
        assert_eq!(
            &reread.document,
            session.document(),
            "{path}: 読み直したら中身が変わった"
        );
        println!(
            "WROTE {name}.ori3 標本={} ids={ids:?} stop={:?} 手数={} 面={} バイト={} length={:.16} width={:.12} position={:.6}",
            sample.name,
            outcome.stop,
            outcome.steps.len(),
            session.faces().len(),
            text.len(),
            outcome.best_gaps.length,
            outcome.best_gaps.width,
            outcome.best_gaps.position,
        );
    }
}

/// task 24記録の離散契約を完全一致で、小数だけを絶対差で照合する。
///
/// 2026-08-28の3標本×4値のJSON往復で観測した最大差は`0.0`、より広い完成参照との
/// 計算残差は最大`1.77e-15`だった。別物を分ける実測最小差は`0.0621320344`なので、
/// `TASK24_GAP_ABS_TOLERANCE = 1e-9`は丸め差に5桁超の余裕を持ち、形の差より7桁超細い。
fn assert_task_24_records_near(
    context: &str,
    got: &Task24CompletionRecord,
    want: &Task24CompletionRecord,
) -> f64 {
    // schema、測定根拠、標本数・ID・順序は既存validatorで完全一致のまま守る。
    assert_task_24_record_valid(got);
    assert_task_24_record_valid(want);

    got.samples
        .iter()
        .zip(&want.samples)
        .map(|(got, want)| {
            assert_task_24_record_near(
                &format!("{context}: {}", got.sample_id),
                got.gaps.as_finish_gaps(),
                want.gaps.as_finish_gaps(),
            )
        })
        .fold(0.0_f64, f64::max)
}
