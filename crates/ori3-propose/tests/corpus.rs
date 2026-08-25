//! 施策3のcorpus schema、read-only fixture契約、製品相当runner。
//!
//! 3-Aで事前固定した30 slotを3-Bで1件ずつ物質化し、製品と同じcore経路で
//! 完成または安全な改善、停止理由、決定性hashをread-onlyで照合する。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Instant;

use ori3_model::{AlignmentTarget, CreasePattern, Document, FoldStep, Paper};
use ori3_propose::{
    body_on_paper, generate, pack, search_to_completion_with_control, verify_search_completion,
    CompletionTolerance, FinishGaps, FinishTarget, FoldGoal, FoldSession, GapWeights, LeafSite,
    Packing, PoseScan, SearchAbort, SearchBudget, SearchControl, SearchWatchdog, Skeleton, TipSite,
    VerifiedPlan,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MANIFEST_NAME: &str = "manifest.json";

/// 小数のbaseline照合とhash量子化に使う幅。
///
/// 既存の製品相当10回検査で反復差の実測最大は0、過去のCI座標差は
/// `1.11e-16`だった。採用値`1e-9`はその約900万倍の丸め余裕を持ち、既存の
/// 完成/未完成の最小分離`0.062132...`より7桁以上細い。座標・角・gapはこの幅で
/// 比較し、個数・ID・接続・種類・順序だけを完全一致で比較する。
const FLOAT_TOL: f64 = 1e-9;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    schema_version: u32,
    corpus_id: String,
    stage: String,
    target_case_count: usize,
    anchor_case_count: usize,
    neutral_case_count: usize,
    pilot_cases_count_toward_target: bool,
    hash_contract: HashContract,
    numeric_policy: NumericPolicy,
    runner_contract: RunnerContract,
    repetitions: Repetitions,
    case_aggregation: CaseAggregation,
    classification_contract: ClassificationContract,
    fixture_contract: FixtureContract,
    public_metrics: Vec<PublicMetric>,
    stratification_plan: Vec<StratumPlan>,
    planned_slots: Vec<PlannedSlot>,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HashContract {
    algorithm: String,
    digest_encoding: String,
    fixture_checksum_scope: String,
    input_normalization: String,
    structure_normalization: String,
    candidate_normalization: String,
    result_normalization: String,
    float_quantum: f64,
    excluded_result_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NumericPolicy {
    gap_abs_tolerance: f64,
    weighted_gap_abs_tolerance: f64,
    coordinate_abs_tolerance: f64,
    angle_abs_tolerance_degrees: f64,
    max_seam_gap: f64,
    max_intersection_pairs_all_poses: usize,
    final_self_intersection_pairs: usize,
    minimum_partial_median_improvement_ratio: f64,
    performance_baseline_fraction_of_gate: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunnerContract {
    product_path: String,
    paper_normalization: String,
    pack_starts: usize,
    packing_max_candidates: usize,
    candidate_execution: String,
    generation_failure: String,
    fold_session_failure: String,
    search_abort: String,
    with_fold_plan: bool,
    search_budget: SearchBudgetContract,
    verification_scan_steps: usize,
    rebuild_scan_steps: usize,
    search_watchdog_millis: u64,
    gap_weights: GapMetric,
    completion_tolerance: GapMetric,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchBudgetContract {
    max_states: usize,
    branch: usize,
    max_depth: usize,
    rank_scan_steps: usize,
    scan_steps: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Repetitions {
    determinism: usize,
    performance_release: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseAggregation {
    completion: String,
    partial: String,
    safety_scope: String,
    no_plan: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationContract {
    position_specified: String,
    position_none: String,
    mixed_position_constraints: String,
    symmetry: String,
    simple: String,
    compound: String,
    required_evidence_field: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureContract {
    root: String,
    ordinary_tests: String,
    regeneration: String,
    absolute_paths_allowed: bool,
    parent_traversal_allowed: bool,
    external_runtime_inputs_allowed: bool,
    checksum_mismatch_allowed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicMetric {
    name: String,
    fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StratumPlan {
    band: String,
    leaf_min: usize,
    leaf_max: usize,
    planned_cases: usize,
    lower_leaf_cases: usize,
    upper_leaf_cases: usize,
    symmetric_cases: usize,
    asymmetric_cases: usize,
    position_specified_cases: usize,
    position_none_cases: usize,
    simple_cases: usize,
    compound_cases: usize,
    must_complete_cases: usize,
    safe_partial_allowed_cases: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedSlot {
    slot: String,
    case_id: String,
    anchor: bool,
    leaf_count: usize,
    symmetry: String,
    position_constraint: String,
    technique_complexity: String,
    expectation_class: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorpusCase {
    id: String,
    display_name: Option<String>,
    planned_slot: Option<String>,
    counts_toward_target: bool,
    source: SourceContract,
    input: InputReference,
    strata: CaseStrata,
    classification_basis: ClassificationBasis,
    target: CaseTarget,
    recorded_current: RecordedCurrent,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceContract {
    kind: String,
    title: String,
    uri: String,
    author: String,
    license_spdx: String,
    license_uri: String,
    attribution: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputReference {
    fixture: String,
    fixture_checksum: Checksum,
    structure_hash: String,
    normalized_input_hash: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checksum {
    algorithm: String,
    digest: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseStrata {
    band: String,
    leaf_count: usize,
    symmetry: String,
    position_constraint: String,
    technique_complexity: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClassificationBasis {
    symmetry_evidence: String,
    position_evidence: String,
    technique_reference: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseTarget {
    class: String,
    criterion: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RecordedOutcomeKind {
    Candidates,
    ExecutionFailure,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetAssessment {
    functional_met: bool,
    safety_met: bool,
    improvement_met: Option<bool>,
    target_met: bool,
    distance_to_target: Option<f64>,
    time_status: String,
    unmet_reasons: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordedCurrent {
    outcome: RecordedOutcomeKind,
    execution_failure: Option<ExecutionFailureExpectation>,
    selected_candidate_index: usize,
    candidate_count: usize,
    candidate_statuses: Vec<String>,
    stop_reasons: Vec<Option<String>>,
    initial_gaps: GapMetric,
    final_gaps: GapMetric,
    initial_weighted_gap: f64,
    final_weighted_gap: f64,
    improvement_absolute: f64,
    improvement_ratio: f64,
    safety: SafetyMetric,
    normalized_candidate_hash: String,
    stop_reason_hash: String,
    normalized_result_hash: String,
    all_returned_plans_safe: bool,
    assessment: TargetAssessment,
    time_budget: TimeBudget,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AbortKind {
    WatchdogExpired,
    Cancelled,
}

impl From<SearchAbort> for AbortKind {
    fn from(abort: SearchAbort) -> Self {
        match abort {
            SearchAbort::WatchdogExpired => Self::WatchdogExpired,
            SearchAbort::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionFailureExpectation {
    phase: String,
    reason: AbortKind,
    normalized_failure_hash: String,
}

#[derive(Clone, Debug, Serialize)]
struct ExecutionFailureContract {
    phase: String,
    reason: AbortKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GapMetric {
    count: f64,
    length: f64,
    width: f64,
    position: f64,
}

impl From<FinishGaps> for GapMetric {
    fn from(gaps: FinishGaps) -> Self {
        Self {
            count: gaps.count,
            length: gaps.length,
            width: gaps.width,
            position: gaps.position,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SafetyMetric {
    all_finite: bool,
    max_seam_gap: f64,
    max_intersection_pairs_all_poses: usize,
    final_self_intersection_pairs: usize,
    layer_warning_count: usize,
    final_warning_count: usize,
    face_count_matches: bool,
    skipped_steps: usize,
    verification_failure: bool,
    report_passed: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TimeBudget {
    search_watchdog_millis: u64,
    measured_debug_elapsed_millis: Option<u64>,
    debug_case_limit_millis: u64,
    measured_release_elapsed_millis: Option<u64>,
    release_case_limit_millis: u64,
    release_corpus_limit_millis: u64,
    limit_status: String,
    ordinary_test_enforces_elapsed: bool,
    basis: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorpusInput {
    schema_version: u32,
    paper: Paper,
    skeleton: Skeleton,
    seed: u64,
    with_fold_plan: bool,
}

#[derive(Clone, Debug, Serialize)]
struct CorpusFoldPlanDetails {
    steps: Vec<FoldStep>,
    cp: CreasePattern,
    planned: usize,
    checked: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CorpusFoldPlan {
    CheckedToFinish {
        #[serde(flatten)]
        details: CorpusFoldPlanDetails,
    },
    Partial {
        #[serde(flatten)]
        details: CorpusFoldPlanDetails,
    },
}

#[derive(Clone, Debug, Serialize)]
struct CorpusCandidate {
    cp: CreasePattern,
    scale: f64,
    violations: usize,
    warnings: Vec<String>,
    sites: Vec<LeafSite>,
    fold_plan: Option<CorpusFoldPlan>,
}

impl CorpusCandidate {
    fn status_tag(&self) -> &'static str {
        match &self.fold_plan {
            Some(CorpusFoldPlan::CheckedToFinish { .. }) => "checked_to_finish",
            Some(CorpusFoldPlan::Partial { .. }) => "partial",
            None => "no_plan",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CandidateMetric {
    status: String,
    stop_reason: String,
    planned: usize,
    checked: usize,
    initial_gaps: GapMetric,
    final_gaps: GapMetric,
    initial_weighted_gap: f64,
    final_weighted_gap: f64,
    improvement_absolute: f64,
    improvement_ratio: f64,
    safety: SafetyMetric,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct CandidatePhaseTiming {
    generate_micros: u64,
    search_micros: u64,
    verify_21_poses_micros: u64,
    rebuild_micros: u64,
}

#[derive(Debug)]
struct CorpusRun {
    candidates: Vec<CorpusCandidate>,
    stop_reasons: Vec<Option<String>>,
    metrics: Vec<Option<CandidateMetric>>,
    phase_timings: Vec<CandidatePhaseTiming>,
}

#[derive(Debug)]
enum CandidateRunError {
    Generation(String),
    SearchAborted(AbortKind),
}

#[derive(Debug)]
enum CorpusRunError {
    Infrastructure(String),
    SearchAborted(AbortKind),
}

#[derive(Serialize)]
struct NormalizedInputContract<'a> {
    input: &'a CorpusInput,
    runner: &'a RunnerContract,
}

#[derive(Serialize)]
struct DeterminismContract<'a> {
    candidates: &'a [CorpusCandidate],
    stop_reasons: &'a [Option<String>],
    metrics: &'a [Option<CandidateMetric>],
}

fn corpus_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("corpus")
}

fn manifest_path() -> PathBuf {
    corpus_fixture_root().join(MANIFEST_NAME)
}

fn load_manifest() -> Result<(Vec<u8>, CorpusManifest), String> {
    let bytes = fs::read(manifest_path()).map_err(|error| format!("manifest読込: {error}"))?;
    let manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("manifest schema不一致: {error}"))?;
    Ok((bytes, manifest))
}

fn fixture_path(relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "corpus外を指すfixture path: {}",
            relative.display()
        ));
    }
    Ok(corpus_fixture_root().join(relative))
}

fn load_input(case: &CorpusCase) -> Result<(Vec<u8>, CorpusInput), String> {
    let path = fixture_path(&case.input.fixture)?;
    let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let input = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{}: input schema不一致: {error}", path.display()))?;
    Ok((bytes, input))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a64(bytes))
}

fn fixture_checksum(bytes: &[u8]) -> Result<String, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("fixtureがUTF-8でない: {error}"))?;
    let normalized = text.replace("\r\n", "\n");
    if normalized.contains('\r') {
        return Err("fixtureに単独CRがある".to_owned());
    }
    Ok(digest(normalized.as_bytes()))
}

fn canonical_text<T: Serialize>(value: &T, quantum: f64) -> Result<String, String> {
    if !quantum.is_finite() || quantum <= 0.0 {
        return Err("hash量子化幅が正の有限値でない".to_owned());
    }
    let value = serde_json::to_value(value).map_err(|error| format!("hash用JSON: {error}"))?;
    let mut text = String::new();
    append_canonical(&value, quantum, &mut text)?;
    Ok(text)
}

fn append_canonical(value: &Value, quantum: f64, text: &mut String) -> Result<(), String> {
    match value {
        Value::Null => text.push('n'),
        Value::Bool(value) => text.push_str(if *value { "b1" } else { "b0" }),
        Value::Number(number) if number.is_f64() => {
            let value = number
                .as_f64()
                .ok_or_else(|| "小数をf64として読めない".to_owned())?;
            let scaled = value / quantum;
            if !value.is_finite() || !scaled.is_finite() {
                return Err(format!("非有限のhash入力: {value}"));
            }
            let rounded = scaled.round();
            let rounded = if rounded == 0.0 { 0.0 } else { rounded };
            text.push('q');
            text.push_str(&format!("{rounded:.0}"));
        }
        Value::Number(number) => {
            text.push('i');
            text.push_str(&number.to_string());
        }
        Value::String(value) => {
            text.push('s');
            text.push_str(
                &serde_json::to_string(value)
                    .map_err(|error| format!("hash文字列のescape: {error}"))?,
            );
        }
        Value::Array(values) => {
            text.push('[');
            for value in values {
                append_canonical(value, quantum, text)?;
                text.push(',');
            }
            text.push(']');
        }
        Value::Object(values) => {
            text.push('{');
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_unstable();
            for key in keys {
                text.push_str(
                    &serde_json::to_string(key)
                        .map_err(|error| format!("hash keyのescape: {error}"))?,
                );
                text.push(':');
                append_canonical(&values[key], quantum, text)?;
                text.push(',');
            }
            text.push('}');
        }
    }
    Ok(())
}

fn normalized_hash<T: Serialize>(value: &T, quantum: f64) -> Result<String, String> {
    Ok(digest(canonical_text(value, quantum)?.as_bytes()))
}

fn quantized(value: f64, quantum: f64) -> Result<String, String> {
    if !value.is_finite() {
        return Err(format!("構造に非有限値がある: {value}"));
    }
    let rounded = (value / quantum).round();
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    Ok(format!("{rounded:.0}"))
}

fn structure_hash(skeleton: &Skeleton, quantum: f64) -> Result<String, String> {
    skeleton
        .validate()
        .map_err(|error| format!("構造hash対象が不正: {error}"))?;
    let root = skeleton
        .root()
        .ok_or_else(|| "構造hash対象に根がない".to_owned())?;
    let length_scale = skeleton
        .nodes
        .iter()
        .filter(|node| node.parent.is_some())
        .map(|node| node.length)
        .fold(0.0_f64, f64::max);
    if !length_scale.is_finite() || length_scale <= 0.0 {
        return Err("構造hashの長さ基準が不正".to_owned());
    }

    fn node_token(
        skeleton: &Skeleton,
        id: u32,
        length_scale: f64,
        quantum: f64,
    ) -> Result<String, String> {
        let node = skeleton
            .node(id)
            .ok_or_else(|| format!("構造hash中に未知の節点{id}"))?;
        let mut children = skeleton
            .nodes
            .iter()
            .filter(|candidate| candidate.parent == Some(id))
            .map(|child| node_token(skeleton, child.id, length_scale, quantum))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_unstable();
        let length = if node.parent.is_none() {
            "root".to_owned()
        } else {
            quantized(node.length / length_scale, quantum)?
        };
        let position = match node.tip_pos_2d {
            Some(position) => format!(
                "{},{}",
                quantized(position.x, quantum)?,
                quantized(position.y, quantum)?
            ),
            None => "none".to_owned(),
        };
        Ok(format!(
            "({length};{};{position};{})",
            quantized(node.width_factor, quantum)?,
            children.join("")
        ))
    }

    let token = node_token(skeleton, root, length_scale, quantum)?;
    Ok(digest(token.as_bytes()))
}

fn point_finite(point: [f64; 2]) -> bool {
    point.into_iter().all(f64::is_finite)
}

fn cp_finite(cp: &CreasePattern) -> bool {
    cp.vertices.iter().all(|vertex| point_finite(vertex.pos))
}

fn packing_finite(packing: &Packing) -> bool {
    packing.scale.is_finite()
        && packing.violation.is_finite()
        && packing
            .centers
            .iter()
            .all(|(_, center)| point_finite(*center))
        && packing
            .circles
            .iter()
            .all(|circle| point_finite(circle.center) && circle.radius.is_finite())
}

fn sites_finite(sites: &[LeafSite]) -> bool {
    sites.iter().all(|site| {
        point_finite(site.circle.center)
            && site.circle.radius.is_finite()
            && site
                .vertex
                .is_none_or(|vertex| point_finite(vertex.pos) && vertex.gap.is_finite())
    })
}

fn alignment_target_finite(target: &AlignmentTarget) -> bool {
    match target {
        AlignmentTarget::Point { p } => point_finite(*p),
        AlignmentTarget::Line { a, b } => point_finite(*a) && point_finite(*b),
    }
}

fn step_finite(step: &FoldStep) -> bool {
    step.drivers.iter().all(|driver| {
        point_finite(driver.a) && point_finite(driver.b) && driver.target_angle_deg.is_finite()
    }) && step
        .layer_order
        .as_ref()
        .is_none_or(|order| order.iter().all(|point| point_finite(*point)))
        && step
            .alignment
            .as_ref()
            .is_none_or(|alignment| alignment.picks.iter().all(alignment_target_finite))
        && step
            .finish_soft
            .is_none_or(|settings| settings.stiffness.is_finite() && settings.pressure.is_finite())
}

fn elapsed_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn calculate_candidate(
    input: &CorpusInput,
    runner: &RunnerContract,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> Result<
    (
        CorpusCandidate,
        Option<CandidateMetric>,
        Option<String>,
        CandidatePhaseTiming,
    ),
    CandidateRunError,
> {
    let generate_started = Instant::now();
    let proposal = generate(&input.skeleton, packing, paper_w, paper_h)
        .map_err(|error| CandidateRunError::Generation(error.to_string()))?;
    let generate_micros = elapsed_micros(generate_started);
    let mut document = Document::new(input.paper.clone());
    document.cp = proposal.cp.clone();
    let session = match FoldSession::new(&document) {
        Ok(session) => session,
        Err(_) => {
            return Ok((
                CorpusCandidate {
                    cp: proposal.cp,
                    scale: packing.scale,
                    violations: proposal.violations,
                    warnings: proposal.warnings,
                    sites: proposal.sites,
                    fold_plan: None,
                },
                None,
                None,
                CandidatePhaseTiming {
                    generate_micros,
                    search_micros: 0,
                    verify_21_poses_micros: 0,
                    rebuild_micros: 0,
                },
            ));
        }
    };
    let goal = FoldGoal {
        target: FinishTarget::from_skeleton(&input.skeleton),
        body: body_on_paper(&input.skeleton, packing, paper_w, paper_h),
        sites: proposal
            .sites
            .iter()
            .map(|site| TipSite {
                leaf_id: site.circle.leaf_id,
                material: site.vertex.map_or(site.circle.center, |vertex| vertex.pos),
            })
            .collect(),
    };
    let budget = SearchBudget {
        max_states: runner.search_budget.max_states,
        branch: runner.search_budget.branch,
        max_depth: runner.search_budget.max_depth,
        rank_scan: PoseScan {
            steps: runner.search_budget.rank_scan_steps,
        },
        scan: PoseScan {
            steps: runner.search_budget.scan_steps,
        },
    };
    let tolerance = CompletionTolerance {
        count: runner.completion_tolerance.count,
        length: runner.completion_tolerance.length,
        width: runner.completion_tolerance.width,
        position: runner.completion_tolerance.position,
    };
    let weights = GapWeights {
        count: runner.gap_weights.count,
        length: runner.gap_weights.length,
        width: runner.gap_weights.width,
        position: runner.gap_weights.position,
    };
    let never_cancelled = || false;
    let control = SearchControl::new(
        SearchWatchdog {
            max_millis: runner.search_watchdog_millis,
        },
        &never_cancelled,
    );
    let search_started = Instant::now();
    let outcome =
        search_to_completion_with_control(&session, &goal, weights, budget, tolerance, &control)
            .map_err(|abort| CandidateRunError::SearchAborted(abort.into()))?;
    let search_micros = elapsed_micros(search_started);
    let stop_reason = outcome.stop.contract_tag().to_owned();
    let planned = outcome.steps.len();
    let verify_started = Instant::now();
    let verified = verify_search_completion(
        &session,
        &outcome,
        &goal,
        weights,
        PoseScan {
            steps: runner.verification_scan_steps,
        },
        tolerance,
    );
    let verify_21_poses_micros = elapsed_micros(verify_started);
    let checked_to_finish = matches!(&verified, VerifiedPlan::CheckedToFinish(_));
    let report = verified.report();

    let rebuild_started = Instant::now();
    let mut walk = session;
    for step in &report.steps {
        let Some(Ok(mv)) = walk.check_move(
            step.id,
            PoseScan {
                steps: runner.rebuild_scan_steps,
            },
        ) else {
            break;
        };
        if walk.apply(&mv).is_err() {
            break;
        }
    }
    let rebuild_micros = elapsed_micros(rebuild_started);
    let folded = walk.document();
    let checked = folded.sequence.len();
    let fold_plan = if checked == 0 {
        None
    } else {
        let details = CorpusFoldPlanDetails {
            steps: folded.sequence.clone(),
            cp: folded.cp.clone(),
            planned,
            checked,
        };
        if checked_to_finish && checked == planned && planned == report.requested {
            Some(CorpusFoldPlan::CheckedToFinish { details })
        } else {
            Some(CorpusFoldPlan::Partial { details })
        }
    };
    let candidate = CorpusCandidate {
        cp: proposal.cp.clone(),
        scale: packing.scale,
        violations: proposal.violations,
        warnings: proposal.warnings.clone(),
        sites: proposal.sites.clone(),
        fold_plan,
    };
    let initial_weighted_gap = report.start_score;
    let final_weighted_gap = report.final_score;
    let improvement_absolute = initial_weighted_gap - final_weighted_gap;
    let improvement_ratio = if initial_weighted_gap > 0.0 {
        improvement_absolute / initial_weighted_gap
    } else {
        0.0
    };
    let safety = SafetyMetric {
        all_finite: packing_finite(packing)
            && cp_finite(&proposal.cp)
            && sites_finite(&proposal.sites)
            && cp_finite(&folded.cp)
            && folded.sequence.iter().all(step_finite)
            && report.start_gaps.all_finite()
            && report.final_gaps.all_finite()
            && report.start_score.is_finite()
            && report.final_score.is_finite()
            && report.final_check.finite
            && report.steps.iter().all(|step| {
                step.line.into_iter().all(point_finite) && step.max_seam_gap.is_finite()
            })
            && report.max_seam_gap.is_finite()
            && report.final_check.max_seam_gap.is_finite(),
        max_seam_gap: report.max_seam_gap,
        max_intersection_pairs_all_poses: report.penetrations,
        final_self_intersection_pairs: report.final_check.penetrations,
        layer_warning_count: report.steps.iter().map(|step| step.layer_warnings).sum(),
        final_warning_count: report.final_check.warnings,
        face_count_matches: report.final_check.faces == report.final_check.expected_faces,
        skipped_steps: report.final_check.skipped,
        verification_failure: report.failure.is_some(),
        report_passed: report.passed(),
    };
    let metric = CandidateMetric {
        status: candidate.status_tag().to_owned(),
        stop_reason: stop_reason.clone(),
        planned,
        checked,
        initial_gaps: report.start_gaps.into(),
        final_gaps: report.final_gaps.into(),
        initial_weighted_gap,
        final_weighted_gap,
        improvement_absolute,
        improvement_ratio,
        safety,
    };
    Ok((
        candidate,
        Some(metric),
        Some(stop_reason),
        CandidatePhaseTiming {
            generate_micros,
            search_micros,
            verify_21_poses_micros,
            rebuild_micros,
        },
    ))
}

fn run_corpus_case(
    input: &CorpusInput,
    runner: &RunnerContract,
) -> Result<CorpusRun, CorpusRunError> {
    if input.schema_version != 1 {
        return Err(CorpusRunError::Infrastructure(format!(
            "未対応input schema: {}",
            input.schema_version
        )));
    }
    if !input.with_fold_plan || !runner.with_fold_plan {
        return Err(CorpusRunError::Infrastructure(
            "corpusはfold plan付き製品経路だけを受け付ける".to_owned(),
        ));
    }
    let long = input.paper.width_mm.max(input.paper.height_mm);
    if !(long > 0.0 && long.is_finite()) {
        return Err(CorpusRunError::Infrastructure(
            "紙寸法が正の有限値でない".to_owned(),
        ));
    }
    input
        .skeleton
        .validate()
        .map_err(|error| CorpusRunError::Infrastructure(format!("pilot骨格が不正: {error}")))?;
    let paper_w = input.paper.width_mm / long;
    let paper_h = input.paper.height_mm / long;
    let packings = pack(
        &input.skeleton,
        paper_w,
        paper_h,
        input.seed,
        runner.pack_starts,
    );
    if packings.is_empty() {
        return Err(CorpusRunError::Infrastructure(
            "製品経路が配置を1件も返さなかった".to_owned(),
        ));
    }
    if packings.len() > runner.packing_max_candidates {
        return Err(CorpusRunError::Infrastructure(format!(
            "配置が製品上限{}件を超えた: {}件",
            runner.packing_max_candidates,
            packings.len(),
        )));
    }

    let planned = thread::scope(|scope| {
        let workers: Vec<_> = packings
            .iter()
            .enumerate()
            .map(|(index, packing)| {
                scope.spawn(move || {
                    match calculate_candidate(input, runner, packing, paper_w, paper_h) {
                        Ok(candidate) => Ok(Some((index, candidate))),
                        Err(CandidateRunError::Generation(_error)) => Ok(None),
                        Err(CandidateRunError::SearchAborted(abort)) => {
                            Err(CorpusRunError::SearchAborted(abort))
                        }
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
            })
            .collect::<Result<Vec<_>, CorpusRunError>>()
    })?;
    let mut calculated: Vec<_> = planned.into_iter().flatten().collect();
    if calculated.is_empty() {
        return Err(CorpusRunError::Infrastructure(
            "製品経路が候補を1件も返さなかった".to_owned(),
        ));
    }
    calculated.sort_by_key(|(index, _)| *index);
    let mut candidates = Vec::with_capacity(calculated.len());
    let mut metrics = Vec::with_capacity(calculated.len());
    let mut stop_reasons = Vec::with_capacity(calculated.len());
    let mut phase_timings = Vec::with_capacity(calculated.len());
    for (_, (candidate, metric, stop_reason, phase_timing)) in calculated {
        candidates.push(candidate);
        metrics.push(metric);
        stop_reasons.push(stop_reason);
        phase_timings.push(phase_timing);
    }
    Ok(CorpusRun {
        candidates,
        stop_reasons,
        metrics,
        phase_timings,
    })
}

fn safety_passes_policy(safety: &SafetyMetric, policy: &NumericPolicy) -> bool {
    safety.all_finite
        && safety.max_seam_gap <= policy.max_seam_gap
        && safety.max_intersection_pairs_all_poses == policy.max_intersection_pairs_all_poses
        && safety.final_self_intersection_pairs == policy.final_self_intersection_pairs
        && safety.layer_warning_count == 0
        && safety.final_warning_count == 0
        && safety.face_count_matches
        && safety.skipped_steps == 0
        && !safety.verification_failure
        && safety.report_passed
}

fn selected_candidate(
    metrics: &[Option<CandidateMetric>],
    policy: &NumericPolicy,
) -> Option<usize> {
    metrics
        .iter()
        .position(|metric| {
            metric.as_ref().is_some_and(|metric| {
                metric.status == "checked_to_finish" && safety_passes_policy(&metric.safety, policy)
            })
        })
        .or_else(|| {
            metrics
                .iter()
                .enumerate()
                .filter_map(|(index, metric)| metric.as_ref().map(|metric| (index, metric)))
                .filter(|(_, metric)| {
                    metric.status == "partial" && safety_passes_policy(&metric.safety, policy)
                })
                .min_by(|left, right| {
                    if (left.1.final_weighted_gap - right.1.final_weighted_gap).abs()
                        <= policy.weighted_gap_abs_tolerance
                    {
                        left.0.cmp(&right.0)
                    } else {
                        left.1
                            .final_weighted_gap
                            .total_cmp(&right.1.final_weighted_gap)
                    }
                })
                .map(|(index, _)| index)
        })
}

fn assert_near(label: &str, actual: f64, expected: f64, tolerance: f64) {
    assert!(
        actual.is_finite() && expected.is_finite(),
        "{label}: 非有限値"
    );
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}

fn assert_gaps_near(label: &str, actual: GapMetric, expected: GapMetric, tolerance: f64) {
    assert_near(
        &format!("{label}.count"),
        actual.count,
        expected.count,
        tolerance,
    );
    assert_near(
        &format!("{label}.length"),
        actual.length,
        expected.length,
        tolerance,
    );
    assert_near(
        &format!("{label}.width"),
        actual.width,
        expected.width,
        tolerance,
    );
    assert_near(
        &format!("{label}.position"),
        actual.position,
        expected.position,
        tolerance,
    );
}

fn assert_safety_policy(actual: &SafetyMetric, policy: &NumericPolicy) {
    assert!(
        safety_passes_policy(actual, policy),
        "corpus安全契約に違反: {actual:?}"
    );
    assert!(actual.all_finite, "座標・角・gapに非有限値がある");
    assert!(
        actual.max_seam_gap <= policy.max_seam_gap,
        "最大seamが上限を超えた: {} > {}",
        actual.max_seam_gap,
        policy.max_seam_gap
    );
    assert_eq!(
        actual.max_intersection_pairs_all_poses, policy.max_intersection_pairs_all_poses,
        "途中姿勢にpenetrationがある"
    );
    assert_eq!(
        actual.final_self_intersection_pairs, policy.final_self_intersection_pairs,
        "終点に自己交差がある"
    );
    assert_eq!(actual.layer_warning_count, 0, "層警告がある");
    assert_eq!(actual.final_warning_count, 0, "終点警告がある");
    assert!(actual.face_count_matches, "終点で面が欠けた");
    assert_eq!(actual.skipped_steps, 0, "飛ばした手順がある");
    assert!(!actual.verification_failure, "21姿勢の再検証が落ちた");
    assert!(actual.report_passed, "VerifyReport::passedがfalse");
}

fn assert_safety_baseline(actual: &SafetyMetric, expected: &SafetyMetric) {
    assert_eq!(actual.all_finite, expected.all_finite);
    assert_near(
        "safety.max_seam_gap",
        actual.max_seam_gap,
        expected.max_seam_gap,
        FLOAT_TOL,
    );
    assert_eq!(
        actual.max_intersection_pairs_all_poses,
        expected.max_intersection_pairs_all_poses
    );
    assert_eq!(
        actual.final_self_intersection_pairs,
        expected.final_self_intersection_pairs
    );
    assert_eq!(actual.layer_warning_count, expected.layer_warning_count);
    assert_eq!(actual.final_warning_count, expected.final_warning_count);
    assert_eq!(actual.face_count_matches, expected.face_count_matches);
    assert_eq!(actual.skipped_steps, expected.skipped_steps);
    assert_eq!(actual.verification_failure, expected.verification_failure);
    assert_eq!(actual.report_passed, expected.report_passed);
}

fn assert_safety(actual: &SafetyMetric, expected: &SafetyMetric, policy: &NumericPolicy) {
    assert_safety_policy(actual, policy);
    assert_safety_baseline(actual, expected);
}

fn assert_expected_class(
    expectation_class: &str,
    metric: &CandidateMetric,
    policy: &NumericPolicy,
) {
    assert_safety_policy(&metric.safety, policy);
    match expectation_class {
        "must_complete" => {
            assert_eq!(metric.status, "checked_to_finish");
            assert!(metric.checked > 0, "完成planの手数が0");
            assert_eq!(metric.checked, metric.planned);
        }
        "safe_partial_allowed" => {
            assert!(
                matches!(metric.status.as_str(), "checked_to_finish" | "partial"),
                "NoPlanはpartial成功にしない"
            );
            assert!(metric.checked > 0, "partial planの手数が0");
            if metric.status == "checked_to_finish" {
                assert_eq!(metric.checked, metric.planned);
            }
            assert!(
                metric.improvement_absolute > policy.weighted_gap_abs_tolerance,
                "partial許容caseが改善していない: {} -> {}",
                metric.initial_weighted_gap,
                metric.final_weighted_gap
            );
            assert!(metric.improvement_ratio > 0.0);
        }
        other => panic!("未知の期待class: {other}"),
    }
}

fn expected_class_passes(
    expectation_class: &str,
    metric: &CandidateMetric,
    policy: &NumericPolicy,
) -> bool {
    if !safety_passes_policy(&metric.safety, policy) {
        return false;
    }
    match expectation_class {
        "must_complete" => {
            metric.status == "checked_to_finish"
                && metric.checked > 0
                && metric.checked == metric.planned
        }
        "safe_partial_allowed" => {
            matches!(metric.status.as_str(), "checked_to_finish" | "partial")
                && metric.checked > 0
                && (metric.status != "checked_to_finish" || metric.checked == metric.planned)
                && metric.improvement_absolute > policy.weighted_gap_abs_tolerance
                && metric.improvement_ratio > 0.0
        }
        _ => false,
    }
}

fn target_criterion(expectation_class: &str) -> &'static str {
    match expectation_class {
        "must_complete" => "at-least-one-checked-to-finish-and-all-returned-plans-safe",
        "safe_partial_allowed" => "safe-verified-plan-with-positive-weighted-gap-improvement",
        other => panic!("未知の期待class: {other}"),
    }
}

fn calculate_recorded_assessment(
    target: &CaseTarget,
    current: &RecordedCurrent,
    policy: &NumericPolicy,
) -> TargetAssessment {
    let selected_status = current
        .candidate_statuses
        .get(current.selected_candidate_index)
        .map(String::as_str);
    let has_usable_plan = current.outcome == RecordedOutcomeKind::Candidates
        && matches!(selected_status, Some("checked_to_finish" | "partial"));
    let safety_met = has_usable_plan
        && current.all_returned_plans_safe
        && safety_passes_policy(&current.safety, policy);
    let improvement_met = (target.class == "safe_partial_allowed").then_some(
        has_usable_plan
            && current.improvement_absolute > policy.weighted_gap_abs_tolerance
            && current.improvement_ratio > 0.0,
    );
    let functional_met = match target.class.as_str() {
        "must_complete" => selected_status == Some("checked_to_finish"),
        "safe_partial_allowed" => has_usable_plan && improvement_met == Some(true),
        other => panic!("未知の期待class: {other}"),
    };
    let target_met = functional_met && safety_met;
    let distance_to_target = has_usable_plan.then_some(current.final_weighted_gap);
    let mut unmet_reasons = Vec::new();
    if current.outcome == RecordedOutcomeKind::ExecutionFailure {
        unmet_reasons.push("execution_failure".to_owned());
    } else if !has_usable_plan {
        unmet_reasons.push("no_usable_plan".to_owned());
    }
    if !functional_met && has_usable_plan {
        unmet_reasons.push(
            if target.class == "must_complete" {
                "not_checked_to_finish"
            } else {
                "non_positive_improvement"
            }
            .to_owned(),
        );
    }
    if !safety_met {
        unmet_reasons.push("safety_not_verified".to_owned());
    }
    TargetAssessment {
        functional_met,
        safety_met,
        improvement_met,
        target_met,
        distance_to_target,
        time_status: "pending_limit_recalibration".to_owned(),
        unmet_reasons,
    }
}

fn assert_recorded_assessment(
    target: &CaseTarget,
    current: &RecordedCurrent,
    policy: &NumericPolicy,
) {
    assert_eq!(target.criterion, target_criterion(&target.class));
    let calculated = calculate_recorded_assessment(target, current, policy);
    assert_eq!(current.assessment.functional_met, calculated.functional_met);
    assert_eq!(current.assessment.safety_met, calculated.safety_met);
    assert_eq!(
        current.assessment.improvement_met,
        calculated.improvement_met
    );
    assert_eq!(current.assessment.target_met, calculated.target_met);
    assert_eq!(current.assessment.time_status, calculated.time_status);
    assert_eq!(current.assessment.unmet_reasons, calculated.unmet_reasons);
    match (
        current.assessment.distance_to_target,
        calculated.distance_to_target,
    ) {
        (Some(actual), Some(expected)) => assert_near(
            "assessment.distance_to_target",
            actual,
            expected,
            policy.weighted_gap_abs_tolerance,
        ),
        (None, None) => {}
        pair => panic!("distance_to_targetの有無が不一致: {pair:?}"),
    }
}

fn distance_relation(recorded: Option<f64>, observed: Option<f64>, tolerance: f64) -> &'static str {
    match (recorded, observed) {
        (Some(before), Some(after)) if after < before - tolerance => "shrunk",
        (Some(before), Some(after)) if after > before + tolerance => "expanded",
        (Some(_), Some(_)) | (None, None) => "unchanged",
        (None, Some(_)) => "shrunk",
        (Some(_), None) => "expanded",
    }
}

/// 折り鶴の初回debug実測27秒超に対して約3.3倍、再測定のcase全体43,890ms
/// （探索は30,000ms watchdog中断）に対して約2.05倍の余裕を取る。
const MAX_DEBUG_CASE_MILLIS: u64 = 90_000;
/// 折り鶴release実測5,339msに対して置いた旧暫定値10,000ms。D-04の隔離10回最大は
/// 9,977ms、E-04は20,316msだったため再較正待ちであり、現在値照合のgateには使わない。
const MAX_RELEASE_CASE_MILLIS: u64 = 10_000;
/// 1件10秒を30件へ適用した旧暫定値300,000ms。保存実測494,180ms、旧schemaの最終
/// 433,004ms、schema v2の全30件431,686msはいずれも超過した。再較正待ちとして
/// 実測と旧値を両方出力するが、現在値照合のgateには使わない。
const MAX_RELEASE_CORPUS_MILLIS: u64 = 300_000;

struct ObservedCase {
    fixture_checksum: String,
    structure_hash: String,
    normalized_input_hash: String,
    elapsed_millis: u64,
    outcome: ObservedOutcome,
}

struct ObservedCandidates {
    normalized_candidate_hash: String,
    stop_reason_hash: String,
    normalized_result_hash: String,
    run: CorpusRun,
}

struct ObservedExecutionFailure {
    contract: ExecutionFailureContract,
    normalized_failure_hash: String,
}

enum ObservedOutcome {
    Candidates(ObservedCandidates),
    ExecutionFailure(ObservedExecutionFailure),
}

fn corpus_run_guard() -> MutexGuard<'static, ()> {
    static RUN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    RUN_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn observe_case(
    input_bytes: &[u8],
    input: &CorpusInput,
    manifest: &CorpusManifest,
) -> Result<ObservedCase, String> {
    let quantum = manifest.hash_contract.float_quantum;
    let fixture_checksum = fixture_checksum(input_bytes)?;
    let input_structure_hash = structure_hash(&input.skeleton, quantum)?;
    let normalized_input_hash = normalized_hash(
        &NormalizedInputContract {
            input,
            runner: &manifest.runner_contract,
        },
        quantum,
    )?;
    let started = Instant::now();
    let run = run_corpus_case(input, &manifest.runner_contract);
    let elapsed_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let outcome = match run {
        Ok(run) => {
            let normalized_candidate_hash = normalized_hash(&run.candidates, quantum)?;
            let stop_reason_hash = normalized_hash(&run.stop_reasons, quantum)?;
            let normalized_result_hash = normalized_hash(
                &DeterminismContract {
                    candidates: &run.candidates,
                    stop_reasons: &run.stop_reasons,
                    metrics: &run.metrics,
                },
                quantum,
            )?;
            ObservedOutcome::Candidates(ObservedCandidates {
                normalized_candidate_hash,
                stop_reason_hash,
                normalized_result_hash,
                run,
            })
        }
        Err(CorpusRunError::SearchAborted(reason)) => {
            let contract = ExecutionFailureContract {
                phase: "search".to_owned(),
                reason,
            };
            let normalized_failure_hash = normalized_hash(&contract, quantum)?;
            ObservedOutcome::ExecutionFailure(ObservedExecutionFailure {
                contract,
                normalized_failure_hash,
            })
        }
        Err(CorpusRunError::Infrastructure(error)) => return Err(error),
    };
    Ok(ObservedCase {
        fixture_checksum,
        structure_hash: input_structure_hash,
        normalized_input_hash,
        elapsed_millis,
        outcome,
    })
}

fn assert_input_strata(case: &CorpusCase, input: &CorpusInput) {
    let leaves = input.skeleton.leaves();
    assert_eq!(leaves.len(), case.strata.leaf_count);
    let specified = leaves.iter().all(|leaf_id| {
        input
            .skeleton
            .node(*leaf_id)
            .is_some_and(|node| node.tip_pos_2d.is_some())
    });
    let omitted = leaves.iter().all(|leaf_id| {
        input
            .skeleton
            .node(*leaf_id)
            .is_some_and(|node| node.tip_pos_2d.is_none())
    });
    match case.strata.position_constraint.as_str() {
        "specified" => assert!(specified, "{}: 全leafの位置が指定されていない", case.id),
        "none" => assert!(omitted, "{}: 位置指定なしleafへ位置が入った", case.id),
        other => panic!("{}: 未知の位置層別: {other}", case.id),
    }
}

fn assert_metric_baseline(
    actual: &CandidateMetric,
    expected: &RecordedCurrent,
    policy: &NumericPolicy,
) {
    assert_gaps_near(
        "initial_gaps",
        actual.initial_gaps,
        expected.initial_gaps,
        policy.gap_abs_tolerance,
    );
    assert_gaps_near(
        "final_gaps",
        actual.final_gaps,
        expected.final_gaps,
        policy.gap_abs_tolerance,
    );
    assert_near(
        "initial_weighted_gap",
        actual.initial_weighted_gap,
        expected.initial_weighted_gap,
        policy.weighted_gap_abs_tolerance,
    );
    assert_near(
        "final_weighted_gap",
        actual.final_weighted_gap,
        expected.final_weighted_gap,
        policy.weighted_gap_abs_tolerance,
    );
    assert_near(
        "improvement_absolute",
        actual.improvement_absolute,
        expected.improvement_absolute,
        policy.weighted_gap_abs_tolerance,
    );
    // 改善率は改善量/初期scoreなので、4 gapのweighted許容を分子・分母へ
    // 1回ずつ伝播したcase固有の境界を使う。ID・種類・候補順は別途完全一致する。
    let expected_start = expected
        .initial_weighted_gap
        .abs()
        .max(policy.weighted_gap_abs_tolerance);
    let ratio_tolerance = policy.weighted_gap_abs_tolerance / expected_start
        + expected.improvement_absolute.abs() * policy.weighted_gap_abs_tolerance
            / expected_start.powi(2);
    assert_near(
        "improvement_ratio",
        actual.improvement_ratio,
        expected.improvement_ratio,
        ratio_tolerance,
    );
    assert_safety_baseline(&actual.safety, &expected.safety);
}

#[derive(Debug)]
struct CaseEvaluation {
    case_id: String,
    expectation_class: String,
    class_pass: bool,
    safety_pass: bool,
    time_pass: bool,
    completed: bool,
    improvement_ratio: Option<f64>,
}

fn assert_time_contract(case_id: &str, expectation: &RecordedCurrent, runner: &RunnerContract) {
    assert_eq!(
        expectation.time_budget.search_watchdog_millis,
        runner.search_watchdog_millis
    );
    assert!(!expectation.time_budget.ordinary_test_enforces_elapsed);
    assert_eq!(
        expectation.time_budget.limit_status,
        "pending_recalibration"
    );
    assert_eq!(
        expectation.time_budget.debug_case_limit_millis,
        MAX_DEBUG_CASE_MILLIS
    );
    assert_eq!(
        expectation.time_budget.release_case_limit_millis,
        MAX_RELEASE_CASE_MILLIS
    );
    assert_eq!(
        expectation.time_budget.release_corpus_limit_millis,
        MAX_RELEASE_CORPUS_MILLIS
    );
    if let Some(elapsed) = expectation.time_budget.measured_debug_elapsed_millis {
        assert!(
            elapsed <= MAX_DEBUG_CASE_MILLIS,
            "{case_id}: debug実測が90秒超"
        );
    }
    assert!(
        expectation
            .time_budget
            .measured_release_elapsed_millis
            .is_some(),
        "{case_id}: release実測が未記録"
    );
}

fn assert_empty_candidate_envelope(expectation: &RecordedCurrent, quantum: f64) {
    let candidates: Vec<CorpusCandidate> = Vec::new();
    let stops: Vec<Option<String>> = Vec::new();
    let metrics: Vec<Option<CandidateMetric>> = Vec::new();
    assert_eq!(expectation.selected_candidate_index, 0);
    assert_eq!(expectation.candidate_count, 0);
    assert!(expectation.candidate_statuses.is_empty());
    assert!(expectation.stop_reasons.is_empty());
    assert_eq!(
        expectation.normalized_candidate_hash,
        normalized_hash(&candidates, quantum).expect("empty candidate hash")
    );
    assert_eq!(
        expectation.stop_reason_hash,
        normalized_hash(&stops, quantum).expect("empty stop hash")
    );
    assert_eq!(
        expectation.normalized_result_hash,
        normalized_hash(
            &DeterminismContract {
                candidates: &candidates,
                stop_reasons: &stops,
                metrics: &metrics,
            },
            quantum,
        )
        .expect("empty result hash")
    );
}

fn assert_recorded_case(case_id: &str, counts_toward_target: bool) -> CaseEvaluation {
    let _guard = corpus_run_guard();
    let manifest_file = manifest_path();
    let (manifest_before, manifest) = load_manifest().expect("manifestを読めない");
    let case = manifest
        .cases
        .iter()
        .find(|case| case.id == case_id)
        .unwrap_or_else(|| panic!("manifestにcaseがない: {case_id}"));
    assert_eq!(case.counts_toward_target, counts_toward_target);
    let input_file = fixture_path(&case.input.fixture).expect("corpus input pathが不正");
    let (input_before, input) = load_input(case).expect("corpus inputを読めない");
    let observed = observe_case(&input_before, &input, &manifest).expect("corpus case実行失敗");

    assert!(
        case.display_name
            .as_deref()
            .is_none_or(|name| !name.trim().is_empty()),
        "{case_id}: display_nameが空文字"
    );
    assert!(
        [
            case.source.kind.as_str(),
            case.source.title.as_str(),
            case.source.uri.as_str(),
            case.source.author.as_str(),
            case.source.license_spdx.as_str(),
            case.source.license_uri.as_str(),
            case.source.attribution.as_str(),
            case.classification_basis.symmetry_evidence.as_str(),
            case.classification_basis.position_evidence.as_str(),
            case.classification_basis.technique_reference.as_str(),
            case.recorded_current.time_budget.basis.as_str(),
        ]
        .into_iter()
        .all(|field| !field.trim().is_empty()),
        "{case_id}: 由来・分類・時間根拠に空欄がある"
    );
    assert_eq!(case.input.fixture_checksum.algorithm, "fnv1a64");
    assert_eq!(
        observed.fixture_checksum,
        case.input.fixture_checksum.digest
    );
    assert_eq!(observed.structure_hash, case.input.structure_hash);
    assert_eq!(
        observed.normalized_input_hash,
        case.input.normalized_input_hash
    );
    assert_input_strata(case, &input);
    assert_time_contract(case_id, &case.recorded_current, &manifest.runner_contract);
    assert_recorded_assessment(
        &case.target,
        &case.recorded_current,
        &manifest.numeric_policy,
    );
    let current_limit = if cfg!(debug_assertions) {
        MAX_DEBUG_CASE_MILLIS
    } else {
        MAX_RELEASE_CASE_MILLIS
    };
    let time_pass = observed.elapsed_millis <= current_limit;
    let expected_execution_failure = case.recorded_current.execution_failure.as_ref();
    let observed_distance = match &observed.outcome {
        ObservedOutcome::Candidates(actual) => {
            selected_candidate(&actual.run.metrics, &manifest.numeric_policy)
                .and_then(|index| actual.run.metrics[index].as_ref())
                .map(|metric| metric.final_weighted_gap)
        }
        ObservedOutcome::ExecutionFailure(_) => None,
    };
    let relation = distance_relation(
        case.recorded_current.assessment.distance_to_target,
        observed_distance,
        manifest.numeric_policy.weighted_gap_abs_tolerance,
    );
    println!(
        "CORPUS_TARGET_DISTANCE id={case_id} recorded={:?} observed={observed_distance:?} relation={relation} target_met_recorded={}",
        case.recorded_current.assessment.distance_to_target,
        case.recorded_current.assessment.target_met,
    );

    let evaluation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match (expected_execution_failure, &observed.outcome) {
            (Some(expected), ObservedOutcome::ExecutionFailure(actual)) => {
                assert_eq!(
                    case.recorded_current.outcome,
                    RecordedOutcomeKind::ExecutionFailure
                );
                assert_eq!(expected.phase, actual.contract.phase);
                assert_eq!(expected.reason, actual.contract.reason);
                assert_eq!(
                    expected.normalized_failure_hash,
                    actual.normalized_failure_hash
                );
                assert_eq!(expected.reason, AbortKind::WatchdogExpired);
                assert_empty_candidate_envelope(
                    &case.recorded_current,
                    manifest.hash_contract.float_quantum,
                );
                println!(
                "CORPUS_CASE id={case_id} elapsed_millis={} baseline=matched outcome=execution_failure phase={} reason={:?} failure_hash={} acceptance=false",
                observed.elapsed_millis,
                actual.contract.phase,
                actual.contract.reason,
                actual.normalized_failure_hash,
            );
                CaseEvaluation {
                    case_id: case_id.to_owned(),
                    expectation_class: case.target.class.clone(),
                    class_pass: false,
                    safety_pass: false,
                    time_pass,
                    completed: false,
                    improvement_ratio: None,
                }
            }
            (None, ObservedOutcome::Candidates(actual)) => {
                assert_eq!(
                    case.recorded_current.outcome,
                    RecordedOutcomeKind::Candidates
                );
                assert_eq!(actual.run.candidates.len(), actual.run.metrics.len());
                assert_eq!(actual.run.candidates.len(), actual.run.stop_reasons.len());
                for (index, metric) in actual.run.metrics.iter().enumerate() {
                    assert_eq!(
                        metric.as_ref().map(|metric| metric.stop_reason.as_str()),
                        actual.run.stop_reasons[index].as_deref(),
                        "metricと候補順の停止理由が食い違う"
                    );
                }
                let statuses: Vec<_> = actual
                    .run
                    .candidates
                    .iter()
                    .map(CorpusCandidate::status_tag)
                    .collect();
                assert_eq!(
                    actual.run.candidates.len(),
                    case.recorded_current.candidate_count
                );
                assert_eq!(statuses, case.recorded_current.candidate_statuses);
                assert_eq!(actual.run.stop_reasons, case.recorded_current.stop_reasons);
                let expected_selected = case.recorded_current.selected_candidate_index;
                let expected_metric = actual
                    .run
                    .metrics
                    .get(expected_selected)
                    .and_then(Option::as_ref)
                    .expect("baseline選択候補にmetricがない");
                assert_metric_baseline(
                    expected_metric,
                    &case.recorded_current,
                    &manifest.numeric_policy,
                );
                assert_eq!(
                    actual.normalized_candidate_hash,
                    case.recorded_current.normalized_candidate_hash
                );
                assert_eq!(
                    actual.stop_reason_hash,
                    case.recorded_current.stop_reason_hash
                );
                assert_eq!(
                    actual.normalized_result_hash,
                    case.recorded_current.normalized_result_hash
                );

                let safety_pass = actual.run.metrics.iter().all(|metric| {
                    metric.as_ref().is_some_and(|metric| {
                        safety_passes_policy(&metric.safety, &manifest.numeric_policy)
                    })
                });
                assert_eq!(safety_pass, case.recorded_current.all_returned_plans_safe);
                let selected = selected_candidate(&actual.run.metrics, &manifest.numeric_policy);
                let metric = selected.and_then(|index| actual.run.metrics[index].as_ref());
                let class_pass = metric.is_some_and(|metric| {
                    expected_class_passes(&case.target.class, metric, &manifest.numeric_policy)
                });
                let completed = metric.is_some_and(|metric| metric.status == "checked_to_finish");
                let improvement_ratio = (case.target.class == "safe_partial_allowed")
                    .then(|| metric.map(|metric| metric.improvement_ratio))
                    .flatten();
                println!(
                "CORPUS_CASE id={case_id} elapsed_millis={} baseline=matched outcome=candidates candidate_hash={} stop_hash={} result_hash={} class_pass={} safety_pass={} time_pass={}",
                observed.elapsed_millis,
                actual.normalized_candidate_hash,
                actual.stop_reason_hash,
                actual.normalized_result_hash,
                class_pass,
                safety_pass,
                time_pass,
            );
                CaseEvaluation {
                    case_id: case_id.to_owned(),
                    expectation_class: case.target.class.clone(),
                    class_pass,
                    safety_pass,
                    time_pass,
                    completed,
                    improvement_ratio,
                }
            }
            (Some(expected), ObservedOutcome::Candidates(_)) => panic!(
                "{case_id}: baselineは{:?}だが候補応答を観測した",
                expected.reason
            ),
            (None, ObservedOutcome::ExecutionFailure(actual)) => panic!(
                "{case_id}: 候補baselineに対して実行失敗を観測した: {} / {:?}",
                actual.contract.phase, actual.contract.reason
            ),
        }
    }));

    let manifest_after = fs::read(&manifest_file).expect("manifest再読込失敗");
    let input_after = fs::read(&input_file).expect("corpus input再読込失敗");
    assert_eq!(
        manifest_after, manifest_before,
        "通常検査がmanifestを変えた"
    );
    assert_eq!(input_after, input_before, "通常検査がinput fixtureを変えた");
    match evaluation {
        Ok(evaluation) => evaluation,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// 3-Bで1件ずつbaselineを確定する専用入口。通常検査からは呼ばない。
///
/// `ORI3_CORPUS_CASE`で明示した1件だけを製品相当経路へ通す。debugは90,000ms、
/// releaseは10,000msを超えても失敗baselineをJSONへ残し、debugだけ90,000msで停止する。実測と両profileの上限、
/// release全30件300,000msを`time_budget.basis`へ残す。fixtureへは書かない。
#[test]
#[ignore = "3-B baselineの明示的な再生成専用"]
fn regenerate_one_corpus_baseline() {
    let _guard = corpus_run_guard();
    let case_id =
        std::env::var("ORI3_CORPUS_CASE").expect("ORI3_CORPUS_CASEで再生成対象を1件だけ指定する");
    let (_, mut manifest) = load_manifest().expect("manifestを読めない");
    let case_index = manifest
        .cases
        .iter()
        .position(|case| case.id == case_id)
        .unwrap_or_else(|| panic!("manifestにcaseがない: {case_id}"));
    assert!(
        manifest.cases[case_index].counts_toward_target,
        "非算入pilotは3-B再生成の対象外"
    );
    let (input_bytes, input) =
        load_input(&manifest.cases[case_index]).expect("corpus inputを読めない");
    if std::env::var("ORI3_CORPUS_INPUT_ONLY").as_deref() == Ok("1") {
        let quantum = manifest.hash_contract.float_quantum;
        let checksum = fixture_checksum(&input_bytes).expect("fixture checksum失敗");
        let input_structure_hash =
            structure_hash(&input.skeleton, quantum).expect("structure hash失敗");
        let input_hash = normalized_hash(
            &NormalizedInputContract {
                input: &input,
                runner: &manifest.runner_contract,
            },
            quantum,
        )
        .expect("normalized input hash失敗");
        let candidates: Vec<CorpusCandidate> = Vec::new();
        let stops: Vec<Option<String>> = Vec::new();
        let metrics: Vec<Option<CandidateMetric>> = Vec::new();
        let candidate_hash =
            normalized_hash(&candidates, quantum).expect("empty candidate hash失敗");
        let stop_hash = normalized_hash(&stops, quantum).expect("empty stop hash失敗");
        let result_hash = normalized_hash(
            &DeterminismContract {
                candidates: &candidates,
                stop_reasons: &stops,
                metrics: &metrics,
            },
            quantum,
        )
        .expect("empty result hash失敗");
        let watchdog_failure_hash = normalized_hash(
            &ExecutionFailureContract {
                phase: "search".to_owned(),
                reason: AbortKind::WatchdogExpired,
            },
            quantum,
        )
        .expect("watchdog failure hash失敗");
        println!(
            "CORPUS_3B_INPUT_CONTRACT fixture_checksum={checksum} structure_hash={input_structure_hash} normalized_input_hash={input_hash} empty_candidate_hash={candidate_hash} empty_stop_hash={stop_hash} empty_result_hash={result_hash} watchdog_failure_hash={watchdog_failure_hash}"
        );
        return;
    }
    let observed = observe_case(&input_bytes, &input, &manifest).expect("corpus case実行失敗");
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let profile_limit = if cfg!(debug_assertions) {
        MAX_DEBUG_CASE_MILLIS
    } else {
        MAX_RELEASE_CASE_MILLIS
    };
    let time_limit_met = observed.elapsed_millis <= profile_limit;
    if cfg!(debug_assertions) {
        assert!(
            time_limit_met,
            "{case_id}: debug所要時間{}msが上限{profile_limit}msを超えた",
            observed.elapsed_millis,
        );
    }
    let expected_class = manifest.cases[case_index].target.class.clone();
    let previous_debug = manifest.cases[case_index]
        .recorded_current
        .time_budget
        .measured_debug_elapsed_millis;
    let previous_release = manifest.cases[case_index]
        .recorded_current
        .time_budget
        .measured_release_elapsed_millis;
    let measured_debug_elapsed_millis = if cfg!(debug_assertions) {
        Some(observed.elapsed_millis)
    } else {
        previous_debug
    };
    let measured_release_elapsed_millis = if cfg!(debug_assertions) {
        previous_release
    } else {
        Some(observed.elapsed_millis)
    };
    let search_watchdog_millis = manifest.runner_contract.search_watchdog_millis;
    let fixture_checksum = observed.fixture_checksum;
    let input_structure_hash = observed.structure_hash;
    let normalized_input_hash = observed.normalized_input_hash;
    let elapsed_millis = observed.elapsed_millis;
    let case = &mut manifest.cases[case_index];
    case.input.fixture_checksum.algorithm = "fnv1a64".to_owned();
    case.input.fixture_checksum.digest = fixture_checksum;
    case.input.structure_hash = input_structure_hash;
    case.input.normalized_input_hash = normalized_input_hash;
    case.recorded_current.time_budget = TimeBudget {
        search_watchdog_millis,
        measured_debug_elapsed_millis,
        debug_case_limit_millis: MAX_DEBUG_CASE_MILLIS,
        measured_release_elapsed_millis,
        release_case_limit_millis: MAX_RELEASE_CASE_MILLIS,
        release_corpus_limit_millis: MAX_RELEASE_CORPUS_MILLIS,
        limit_status: "pending_recalibration".to_owned(),
        ordinary_test_enforces_elapsed: false,
        basis: format!(
            "Stage 3-B one-case measurements: debug={} with a 90000ms limit; release={} with a 10000ms limit; the 30-case release limit is 300000ms. The limits come from the crane's initial debug observation above 27000ms and the existing release-speed evidence; stage 3-C will replace these single-run observations with five release runs.",
            measured_debug_elapsed_millis.map_or("not recorded".to_owned(), |value| format!("{value}ms")),
            measured_release_elapsed_millis.map_or("not recorded".to_owned(), |value| format!("{value}ms")),
        ),
    };
    match observed.outcome {
        ObservedOutcome::Candidates(actual) => {
            let candidate_json = serde_json::to_string(&actual.run.candidates)
                .expect("candidate diagnostic JSONを作れない");
            // exact hashは今回の原因調査用で、座標・角・gapの検査境界には使わない。
            // 通常baselineは引き続き1e-9量子化hashと許容差比較を正本にする。
            let exact_candidate_json_hash = digest(candidate_json.as_bytes());
            let individual_candidate_hashes: Vec<_> = actual
                .run
                .candidates
                .iter()
                .map(|candidate| {
                    normalized_hash(candidate, manifest.hash_contract.float_quantum)
                        .expect("individual candidate hashを作れない")
                })
                .collect();
            println!(
                "CORPUS_3B_DETERMINISM normalized_candidate_hash={} exact_candidate_json_hash={} individual_candidate_hashes={} stop_hash={} result_hash={}",
                actual.normalized_candidate_hash,
                exact_candidate_json_hash,
                serde_json::to_string(&individual_candidate_hashes)
                    .expect("candidate hash list JSONを作れない"),
                actual.stop_reason_hash,
                actual.normalized_result_hash,
            );
            println!(
                "CORPUS_3B_METRICS={}",
                serde_json::to_string(&actual.run.metrics).expect("metric JSONを作れない")
            );
            let all_returned_plans_safe = actual.run.metrics.iter().all(|metric| {
                metric.as_ref().is_some_and(|metric| {
                    safety_passes_policy(&metric.safety, &manifest.numeric_policy)
                })
            });
            let selected = selected_candidate(&actual.run.metrics, &manifest.numeric_policy)
                .or_else(|| actual.run.metrics.iter().position(Option::is_some))
                .expect("metricを持つ候補がない");
            let metric = actual.run.metrics[selected]
                .as_ref()
                .expect("選択候補にmetricがない");
            let expectation_met =
                expected_class_passes(&expected_class, metric, &manifest.numeric_policy)
                    && all_returned_plans_safe;
            let statuses = actual
                .run
                .candidates
                .iter()
                .map(CorpusCandidate::status_tag)
                .map(str::to_owned)
                .collect();
            case.recorded_current.selected_candidate_index = selected;
            case.recorded_current.candidate_count = actual.run.candidates.len();
            case.recorded_current.candidate_statuses = statuses;
            case.recorded_current.stop_reasons = actual.run.stop_reasons;
            case.recorded_current.initial_gaps = metric.initial_gaps;
            case.recorded_current.final_gaps = metric.final_gaps;
            case.recorded_current.initial_weighted_gap = metric.initial_weighted_gap;
            case.recorded_current.final_weighted_gap = metric.final_weighted_gap;
            case.recorded_current.improvement_absolute = metric.improvement_absolute;
            case.recorded_current.improvement_ratio = metric.improvement_ratio;
            case.recorded_current.safety = metric.safety.clone();
            case.recorded_current.normalized_candidate_hash = actual.normalized_candidate_hash;
            case.recorded_current.stop_reason_hash = actual.stop_reason_hash;
            case.recorded_current.normalized_result_hash = actual.normalized_result_hash;
            case.recorded_current.outcome = RecordedOutcomeKind::Candidates;
            case.recorded_current.execution_failure = None;
            case.recorded_current.all_returned_plans_safe = all_returned_plans_safe;
            case.recorded_current.assessment = calculate_recorded_assessment(
                &case.target,
                &case.recorded_current,
                &manifest.numeric_policy,
            );
            println!(
                "CORPUS_3B_PHASE_TIMINGS={}",
                serde_json::to_string(&actual.run.phase_timings)
                    .expect("phase timing JSONを作れない")
            );
            println!(
                "CORPUS_3B_TARGET_STATUS case={} class={} selected_status={} all_returned_plans_safe={} target_met={} profile={} provisional_time_limit_met={} elapsed_millis={}",
                case_id,
                expected_class,
                metric.status,
                all_returned_plans_safe,
                expectation_met,
                profile,
                time_limit_met,
                elapsed_millis,
            );
        }
        ObservedOutcome::ExecutionFailure(actual) => {
            let failure = ExecutionFailureExpectation {
                phase: actual.contract.phase,
                reason: actual.contract.reason,
                normalized_failure_hash: actual.normalized_failure_hash,
            };
            case.recorded_current.selected_candidate_index = 0;
            case.recorded_current.candidate_count = 0;
            case.recorded_current.candidate_statuses.clear();
            case.recorded_current.stop_reasons.clear();
            case.recorded_current.initial_gaps = GapMetric {
                count: 0.0,
                length: 0.0,
                width: 0.0,
                position: 0.0,
            };
            case.recorded_current.final_gaps = case.recorded_current.initial_gaps;
            case.recorded_current.initial_weighted_gap = 0.0;
            case.recorded_current.final_weighted_gap = 0.0;
            case.recorded_current.improvement_absolute = 0.0;
            case.recorded_current.improvement_ratio = 0.0;
            case.recorded_current.safety = SafetyMetric {
                all_finite: false,
                max_seam_gap: 0.0,
                max_intersection_pairs_all_poses: 0,
                final_self_intersection_pairs: 0,
                layer_warning_count: 0,
                final_warning_count: 0,
                face_count_matches: false,
                skipped_steps: 0,
                verification_failure: true,
                report_passed: false,
            };
            let candidates: Vec<CorpusCandidate> = Vec::new();
            let stops: Vec<Option<String>> = Vec::new();
            let metrics: Vec<Option<CandidateMetric>> = Vec::new();
            case.recorded_current.normalized_candidate_hash =
                normalized_hash(&candidates, manifest.hash_contract.float_quantum)
                    .expect("empty candidate hash");
            case.recorded_current.stop_reason_hash =
                normalized_hash(&stops, manifest.hash_contract.float_quantum)
                    .expect("empty stop hash");
            case.recorded_current.normalized_result_hash = normalized_hash(
                &DeterminismContract {
                    candidates: &candidates,
                    stop_reasons: &stops,
                    metrics: &metrics,
                },
                manifest.hash_contract.float_quantum,
            )
            .expect("empty result hash");
            case.recorded_current.outcome = RecordedOutcomeKind::ExecutionFailure;
            case.recorded_current.execution_failure = Some(failure.clone());
            case.recorded_current.all_returned_plans_safe = false;
            case.recorded_current.assessment = calculate_recorded_assessment(
                &case.target,
                &case.recorded_current,
                &manifest.numeric_policy,
            );
            println!(
                "CORPUS_3B_TARGET_STATUS case={} class={} outcome=execution_failure target_met=false profile={} provisional_time_limit_met={} elapsed_millis={}",
                case_id,
                expected_class,
                profile,
                time_limit_met,
                elapsed_millis,
            );
            println!(
                "CORPUS_3B_EXECUTION_FAILURE_JSON={}",
                serde_json::to_string(&failure).expect("execution failure JSONを作れない")
            );
        }
    }
    println!(
        "CORPUS_3B_CASE_JSON={}",
        serde_json::to_string(case).expect("case baseline JSONを作れない")
    );
}

/// 通常のdebug検査ではこの名前を`--skip`し、release jobだけで走らせる。
///
/// 折り鶴のdebug初回実測は27秒超で、通常debug jobには置けない。releaseで実行し、
/// 保存した現在値だけを照合する。旧10秒/300秒値は再較正待ちの観測欄として残し、
/// target未達件数とともに出力するが、このtestの合否境界にはしない。
#[test]
fn corpus_all_thirty_cases_match_recorded_current() {
    let (_, manifest) = load_manifest().expect("manifestを読めない");
    let case_ids: Vec<_> = manifest
        .planned_slots
        .iter()
        .map(|slot| slot.case_id.clone())
        .collect();
    assert_eq!(case_ids.len(), 30);
    let partial_expected_count = manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
        .filter(|case| case.target.class == "safe_partial_allowed")
        .count();
    assert_eq!(partial_expected_count, 18);
    let started = Instant::now();
    let mut baseline_mismatches = Vec::new();
    let mut evaluations = Vec::new();
    for case_id in case_ids {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_recorded_case(&case_id, true)
        }));
        match result {
            Ok(evaluation) => evaluations.push(evaluation),
            Err(_) => baseline_mismatches.push(case_id),
        }
    }
    let elapsed_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut target_unmet: Vec<_> = evaluations
        .iter()
        .filter(|evaluation| !(evaluation.class_pass && evaluation.safety_pass))
        .map(|evaluation| evaluation.case_id.clone())
        .collect();
    target_unmet.extend(baseline_mismatches.iter().cloned());
    target_unmet.sort_unstable();
    target_unmet.dedup();

    let completed = evaluations
        .iter()
        .filter(|evaluation| evaluation.completed)
        .count();
    let class_passed = evaluations
        .iter()
        .filter(|evaluation| evaluation.class_pass)
        .count();
    let safety_passed = evaluations
        .iter()
        .filter(|evaluation| evaluation.safety_pass)
        .count();
    let time_passed = evaluations
        .iter()
        .filter(|evaluation| evaluation.time_pass)
        .count();
    let mut partial_ratios: Vec<_> = evaluations
        .iter()
        .filter(|evaluation| evaluation.expectation_class == "safe_partial_allowed")
        .filter_map(|evaluation| evaluation.improvement_ratio)
        .filter(|ratio| ratio.is_finite())
        .collect();
    partial_ratios.sort_by(f64::total_cmp);
    // §7.4(4)の母集団は事前classがsafe_partial_allowedの18件すべて。欠測を0へ
    // 置換せず、18値が揃った場合だけ9番目・10番目の平均を正式な中央値にする。
    let partial_median = (partial_ratios.len() == partial_expected_count).then(|| {
        (partial_ratios[partial_expected_count / 2 - 1]
            + partial_ratios[partial_expected_count / 2])
            / 2.0
    });
    let target_met = evaluations
        .iter()
        .filter(|evaluation| evaluation.class_pass && evaluation.safety_pass)
        .count();
    println!(
        "RECORDED_CURRENT matched={} mismatched={} mismatch_cases={baseline_mismatches:?}",
        evaluations.len(),
        baseline_mismatches.len(),
    );
    println!(
        "TARGET_STATUS met={} unmet={} completed={} class_met={} safety_observed={} partial_ratio_values={}/{} partial_median={partial_median:?} unmet_cases={target_unmet:?}",
        target_met,
        target_unmet.len(),
        completed,
        class_passed,
        safety_passed,
        partial_ratios.len(),
        partial_expected_count,
    );
    println!(
        "TARGET_DISTANCE unchanged={} changed_or_unavailable={}",
        evaluations.len(),
        baseline_mismatches.len(),
    );
    println!(
        "PERFORMANCE_OBSERVATION elapsed_millis={elapsed_millis} cases=30 provisional_case_limit_millis={MAX_RELEASE_CASE_MILLIS} provisional_corpus_limit_millis={MAX_RELEASE_CORPUS_MILLIS} cases_within_provisional_limit={time_passed} limit_status=pending_recalibration enforced=false",
    );
    // この検査のgreenは「記録済み現在値の再現」だけを表す。目標未達と暫定時間超過は
    // 上の独立集計に必ず残し、targetをbaselineへ引き下げない。
    assert!(
        baseline_mismatches.is_empty(),
        "recorded-current mismatch cases: {baseline_mismatches:?}"
    );
}

#[test]
fn manifest_materializes_thirty_stratified_cases_without_changing_the_plan() {
    let (_, manifest) = load_manifest().expect("manifestを読めない");
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.corpus_id, "proposal-benchmark-corpus-v2");
    assert_eq!(manifest.stage, "3-B");
    assert_eq!(manifest.target_case_count, 30);
    assert_eq!(manifest.anchor_case_count, 4);
    assert_eq!(manifest.neutral_case_count, 26);
    assert!(!manifest.pilot_cases_count_toward_target);
    assert_eq!(manifest.repetitions.determinism, 10);
    assert_eq!(manifest.repetitions.performance_release, 5);
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.counts_toward_target)
            .filter(|case| {
                case.recorded_current.outcome == RecordedOutcomeKind::ExecutionFailure
            })
            .count(),
        9
    );
    assert_eq!(manifest.hash_contract.algorithm, "fnv1a64");
    assert_eq!(manifest.hash_contract.digest_encoding, "lowercase-hex-16");
    assert_eq!(
        manifest.hash_contract.fixture_checksum_scope,
        "utf8-text-crlf-normalized-to-lf"
    );
    assert_eq!(
        manifest.hash_contract.input_normalization,
        "typed-input-plus-runner-contract-canonical-json-v1"
    );
    assert_eq!(
        manifest.hash_contract.candidate_normalization,
        "product-candidate-canonical-json-v1"
    );
    assert_eq!(
        manifest.hash_contract.result_normalization,
        "benchmark-result-envelope-canonical-json-v1"
    );
    assert_eq!(
        manifest.hash_contract.structure_normalization,
        "rooted-tree-node-order-id-and-uniform-scale-independent-v1"
    );
    assert_eq!(
        manifest.hash_contract.excluded_result_fields,
        ["elapsed_millis", "machine"]
    );
    assert_near(
        "hash float quantum",
        manifest.hash_contract.float_quantum,
        FLOAT_TOL,
        f64::EPSILON,
    );
    assert_near(
        "gap tolerance",
        manifest.numeric_policy.gap_abs_tolerance,
        FLOAT_TOL,
        f64::EPSILON,
    );
    let derived_weighted_tolerance = 2.0
        * (GapWeights::DEFAULT.count
            + GapWeights::DEFAULT.length
            + GapWeights::DEFAULT.width
            + GapWeights::DEFAULT.position)
        * manifest.numeric_policy.gap_abs_tolerance;
    assert_near(
        "weighted gap tolerance",
        manifest.numeric_policy.weighted_gap_abs_tolerance,
        derived_weighted_tolerance,
        f64::EPSILON,
    );
    assert_near(
        "coordinate tolerance",
        manifest.numeric_policy.coordinate_abs_tolerance,
        FLOAT_TOL,
        f64::EPSILON,
    );
    assert_near(
        "angle tolerance",
        manifest.numeric_policy.angle_abs_tolerance_degrees,
        FLOAT_TOL,
        f64::EPSILON,
    );
    assert_near(
        "performance baseline fraction",
        manifest
            .numeric_policy
            .performance_baseline_fraction_of_gate,
        0.8,
        f64::EPSILON,
    );
    assert_near(
        "maximum seam gap",
        manifest.numeric_policy.max_seam_gap,
        1e-6,
        f64::EPSILON,
    );
    assert_eq!(manifest.numeric_policy.max_intersection_pairs_all_poses, 0);
    assert_eq!(manifest.numeric_policy.final_self_intersection_pairs, 0);
    assert_near(
        "partial median improvement",
        manifest
            .numeric_policy
            .minimum_partial_median_improvement_ratio,
        0.2,
        f64::EPSILON,
    );
    assert_eq!(manifest.runner_contract.product_path, "proposal_generate");
    assert_eq!(
        manifest.runner_contract.paper_normalization,
        "long-edge-equals-1"
    );
    assert_eq!(manifest.runner_contract.pack_starts, 8);
    assert_eq!(manifest.runner_contract.packing_max_candidates, 4);
    assert_eq!(
        manifest.runner_contract.candidate_execution,
        "parallel-preserve-packing-order"
    );
    assert_eq!(
        manifest.runner_contract.generation_failure,
        "omit-candidate; fail-case-if-all"
    );
    assert_eq!(
        manifest.runner_contract.fold_session_failure,
        "return-candidate-without-plan-and-stop; corpus-gate-fails"
    );
    assert_eq!(manifest.runner_contract.search_abort, "fail-case");
    assert!(manifest.runner_contract.with_fold_plan);
    assert_eq!(manifest.runner_contract.search_budget.max_states, 2);
    assert_eq!(manifest.runner_contract.search_budget.branch, 2);
    assert_eq!(
        manifest.runner_contract.search_budget.max_depth,
        SearchBudget::DEFAULT.max_depth
    );
    assert_eq!(
        manifest.runner_contract.search_budget.rank_scan_steps,
        SearchBudget::DEFAULT.rank_scan.steps
    );
    assert_eq!(
        manifest.runner_contract.search_budget.scan_steps,
        SearchBudget::DEFAULT.scan.steps
    );
    assert_eq!(
        manifest.runner_contract.verification_scan_steps,
        PoseScan::DEFAULT.steps
    );
    assert_eq!(manifest.runner_contract.rebuild_scan_steps, 0);
    assert_eq!(manifest.runner_contract.search_watchdog_millis, 30_000);
    assert_gaps_near(
        "gap_weights",
        manifest.runner_contract.gap_weights,
        GapMetric {
            count: GapWeights::DEFAULT.count,
            length: GapWeights::DEFAULT.length,
            width: GapWeights::DEFAULT.width,
            position: GapWeights::DEFAULT.position,
        },
        f64::EPSILON,
    );
    assert_gaps_near(
        "completion_tolerance",
        manifest.runner_contract.completion_tolerance,
        GapMetric {
            count: CompletionTolerance::DEFAULT.count,
            length: CompletionTolerance::DEFAULT.length,
            width: CompletionTolerance::DEFAULT.width,
            position: CompletionTolerance::DEFAULT.position,
        },
        f64::EPSILON,
    );
    assert_eq!(
        manifest.classification_contract.position_specified,
        "every leaf has tip_pos_2d"
    );
    assert_eq!(
        manifest.classification_contract.position_none,
        "every leaf omits tip_pos_2d"
    );
    assert_eq!(
        manifest.classification_contract.mixed_position_constraints,
        "rejected"
    );
    assert_eq!(
        manifest.classification_contract.required_evidence_field,
        "classification_basis"
    );
    assert!(manifest
        .classification_contract
        .symmetry
        .contains("rooted-tree"));
    assert!(manifest.classification_contract.simple.contains("ordinary"));
    assert!(manifest
        .classification_contract
        .compound
        .contains("compound"));
    assert!(manifest
        .case_aggregation
        .completion
        .contains("any-candidate"));
    assert!(manifest
        .case_aggregation
        .partial
        .contains("lowest-final-weighted-gap"));
    assert_eq!(manifest.case_aggregation.safety_scope, "all-returned-plans");
    assert_eq!(
        manifest.case_aggregation.no_plan,
        "case-failure-if-no-safe-plan"
    );
    assert_eq!(
        manifest.fixture_contract.root,
        "crates/ori3-propose/tests/fixtures/corpus"
    );
    assert_eq!(manifest.fixture_contract.ordinary_tests, "read-only");
    assert_eq!(
        manifest.fixture_contract.regeneration,
        "separate-ignored-test-only"
    );
    assert!(!manifest.fixture_contract.absolute_paths_allowed);
    assert!(!manifest.fixture_contract.parent_traversal_allowed);
    assert!(!manifest.fixture_contract.external_runtime_inputs_allowed);
    assert!(!manifest.fixture_contract.checksum_mismatch_allowed);

    let metric_names: BTreeSet<_> = manifest
        .public_metrics
        .iter()
        .map(|metric| metric.name.as_str())
        .collect();
    assert_eq!(
        metric_names,
        BTreeSet::from([
            "completion_rate",
            "weighted_gap_improvement",
            "safety",
            "stop_reasons",
            "elapsed_millis",
            "determinism_hash",
        ])
    );
    let metric_fields = |name: &str| {
        manifest
            .public_metrics
            .iter()
            .find(|metric| metric.name == name)
            .unwrap_or_else(|| panic!("metricがない: {name}"))
            .fields
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        metric_fields("completion_rate"),
        ["status", "checked", "planned"]
    );
    assert_eq!(
        metric_fields("weighted_gap_improvement"),
        [
            "initial_gaps",
            "final_gaps",
            "initial_weighted_gap",
            "final_weighted_gap",
            "improvement_absolute",
            "improvement_ratio",
        ]
    );
    assert_eq!(
        metric_fields("safety"),
        [
            "all_finite",
            "max_seam_gap",
            "max_intersection_pairs_all_poses",
            "final_self_intersection_pairs",
            "layer_warning_count",
            "final_warning_count",
            "face_count_matches",
            "skipped_steps",
            "verification_failure",
            "report_passed",
        ]
    );
    assert_eq!(
        metric_fields("stop_reasons"),
        [
            "candidate_order",
            "contract_tag",
            "null_when_search_not_started",
        ]
    );
    assert_eq!(
        metric_fields("elapsed_millis"),
        ["case_total", "release_median", "release_p95"]
    );
    assert_eq!(
        metric_fields("determinism_hash"),
        [
            "normalized_candidate_hash",
            "stop_reason_hash",
            "normalized_result_hash",
            "normalized_failure_hash",
        ]
    );

    assert_eq!(manifest.stratification_plan.len(), 5);
    assert_eq!(manifest.planned_slots.len(), 30);
    let mut slot_ids = BTreeSet::new();
    let mut case_ids = BTreeSet::new();
    for slot in &manifest.planned_slots {
        assert!(
            slot_ids.insert(slot.slot.as_str()),
            "slot重複: {}",
            slot.slot
        );
        assert!(
            case_ids.insert(slot.case_id.as_str()),
            "case ID重複: {}",
            slot.case_id
        );
        assert!((3..=12).contains(&slot.leaf_count));
        assert!(matches!(slot.symmetry.as_str(), "symmetric" | "asymmetric"));
        assert!(matches!(
            slot.position_constraint.as_str(),
            "specified" | "none"
        ));
        assert!(matches!(
            slot.technique_complexity.as_str(),
            "simple" | "compound"
        ));
        assert!(matches!(
            slot.expectation_class.as_str(),
            "must_complete" | "safe_partial_allowed"
        ));
    }

    for (index, plan) in manifest.stratification_plan.iter().enumerate() {
        let expected_band = char::from(b'A' + index as u8).to_string();
        assert_eq!(plan.band, expected_band);
        assert_eq!(plan.leaf_min, 3 + index * 2);
        assert_eq!(plan.leaf_max, plan.leaf_min + 1);
        assert_eq!(plan.planned_cases, 6);
        assert_eq!(plan.lower_leaf_cases, 3);
        assert_eq!(plan.upper_leaf_cases, 3);
        assert_eq!(plan.symmetric_cases, 3);
        assert_eq!(plan.asymmetric_cases, 3);
        assert_eq!(plan.position_specified_cases, 3);
        assert_eq!(plan.position_none_cases, 3);
        assert_eq!(plan.simple_cases, 3);
        assert_eq!(plan.compound_cases, 3);
        assert_eq!(
            plan.must_complete_cases + plan.safe_partial_allowed_cases,
            plan.planned_cases
        );
        let prefix = format!("{}-", plan.band);
        let slots: Vec<_> = manifest
            .planned_slots
            .iter()
            .filter(|slot| slot.slot.starts_with(&prefix))
            .collect();
        assert_eq!(slots.len(), plan.planned_cases);
        let count = |predicate: &dyn Fn(&PlannedSlot) -> bool| {
            slots.iter().filter(|slot| predicate(slot)).count()
        };
        assert_eq!(
            count(&|slot| slot.leaf_count == plan.leaf_min),
            plan.lower_leaf_cases
        );
        assert_eq!(
            count(&|slot| slot.leaf_count == plan.leaf_max),
            plan.upper_leaf_cases
        );
        assert_eq!(
            count(&|slot| slot.symmetry == "symmetric"),
            plan.symmetric_cases
        );
        assert_eq!(
            count(&|slot| slot.symmetry == "asymmetric"),
            plan.asymmetric_cases
        );
        assert_eq!(
            count(&|slot| slot.position_constraint == "specified"),
            plan.position_specified_cases
        );
        assert_eq!(
            count(&|slot| slot.position_constraint == "none"),
            plan.position_none_cases
        );
        assert_eq!(
            count(&|slot| slot.technique_complexity == "simple"),
            plan.simple_cases
        );
        assert_eq!(
            count(&|slot| slot.technique_complexity == "compound"),
            plan.compound_cases
        );
        assert_eq!(
            count(&|slot| slot.expectation_class == "must_complete"),
            plan.must_complete_cases
        );
        assert_eq!(
            count(&|slot| slot.expectation_class == "safe_partial_allowed"),
            plan.safe_partial_allowed_cases
        );
    }

    let global_count = |predicate: &dyn Fn(&PlannedSlot) -> bool| {
        manifest
            .planned_slots
            .iter()
            .filter(|slot| predicate(slot))
            .count()
    };
    assert_eq!(global_count(&|slot| slot.symmetry == "symmetric"), 15);
    assert_eq!(global_count(&|slot| slot.symmetry == "asymmetric"), 15);
    assert_eq!(
        global_count(&|slot| slot.position_constraint == "specified"),
        15
    );
    assert_eq!(global_count(&|slot| slot.position_constraint == "none"), 15);
    assert_eq!(
        global_count(&|slot| slot.technique_complexity == "simple"),
        15
    );
    assert_eq!(
        global_count(&|slot| slot.technique_complexity == "compound"),
        15
    );
    assert_eq!(
        global_count(&|slot| slot.expectation_class == "must_complete"),
        12
    );
    for leaves in 3..=12 {
        assert_eq!(global_count(&|slot| slot.leaf_count == leaves), 3);
    }

    let anchors: BTreeSet<_> = manifest
        .planned_slots
        .iter()
        .filter(|slot| slot.anchor)
        .map(|slot| slot.case_id.as_str())
        .collect();
    assert_eq!(
        anchors,
        BTreeSet::from(["crane", "yakko", "frog", "bird-base"])
    );
    assert_eq!(global_count(&|slot| slot.anchor), 4);
    assert_eq!(global_count(&|slot| !slot.anchor), 26);
    assert!(manifest
        .planned_slots
        .iter()
        .filter(|slot| !slot.anchor)
        .all(|slot| slot.case_id.starts_with("leaves-")));
    assert_eq!(manifest.cases.len(), 31);
    let mut materialized_slots = BTreeSet::new();
    let mut materialized_structures = BTreeSet::new();
    let mut materialized_case_ids = BTreeSet::new();
    for case in &manifest.cases {
        assert!(
            materialized_case_ids.insert(case.id.as_str()),
            "materialized case ID重複: {}",
            case.id
        );
        if case.counts_toward_target {
            let planned_slot = case
                .planned_slot
                .as_ref()
                .and_then(|slot_id| {
                    manifest
                        .planned_slots
                        .iter()
                        .find(|slot| &slot.slot == slot_id)
                })
                .unwrap_or_else(|| panic!("{}: planned_slotが未登録", case.id));
            assert!(
                materialized_slots.insert(planned_slot.slot.as_str()),
                "複数caseが同じplanned_slotを使った: {}",
                planned_slot.slot
            );
            assert!(
                materialized_structures.insert(case.input.structure_hash.as_str()),
                "同じ構造hashを別作品として数えた: {}",
                case.input.structure_hash
            );
            assert_eq!(case.id, planned_slot.case_id);
            assert_eq!(case.strata.leaf_count, planned_slot.leaf_count);
            assert_eq!(case.strata.symmetry, planned_slot.symmetry);
            assert_eq!(
                case.strata.position_constraint,
                planned_slot.position_constraint
            );
            assert_eq!(
                case.strata.technique_complexity,
                planned_slot.technique_complexity
            );
            assert_eq!(case.target.class, planned_slot.expectation_class);
            assert!(planned_slot.slot.starts_with(&case.strata.band));
            let (input_bytes, input) = load_input(case)
                .unwrap_or_else(|error| panic!("{}: fixtureを読めない: {error}", case.id));
            assert_eq!(
                fixture_checksum(&input_bytes).expect("fixture checksum失敗"),
                case.input.fixture_checksum.digest,
                "{}: fixture checksum不一致",
                case.id
            );
            assert_eq!(
                structure_hash(&input.skeleton, manifest.hash_contract.float_quantum)
                    .expect("structure hash失敗"),
                case.input.structure_hash,
                "{}: structure hash不一致",
                case.id
            );
            assert_eq!(
                normalized_hash(
                    &NormalizedInputContract {
                        input: &input,
                        runner: &manifest.runner_contract,
                    },
                    manifest.hash_contract.float_quantum,
                )
                .expect("normalized input hash失敗"),
                case.input.normalized_input_hash,
                "{}: normalized input hash不一致",
                case.id
            );
            assert_input_strata(case, &input);
            assert_recorded_assessment(
                &case.target,
                &case.recorded_current,
                &manifest.numeric_policy,
            );
            match case.recorded_current.execution_failure.as_ref() {
                Some(failure) => {
                    assert_eq!(
                        case.recorded_current.outcome,
                        RecordedOutcomeKind::ExecutionFailure
                    );
                    assert_eq!(failure.phase, "search");
                    assert_eq!(failure.reason, AbortKind::WatchdogExpired);
                    assert_eq!(
                        failure.normalized_failure_hash,
                        normalized_hash(
                            &ExecutionFailureContract {
                                phase: failure.phase.clone(),
                                reason: failure.reason,
                            },
                            manifest.hash_contract.float_quantum,
                        )
                        .expect("execution failure hash失敗"),
                        "{}: execution failure hash不一致",
                        case.id
                    );
                    assert_empty_candidate_envelope(
                        &case.recorded_current,
                        manifest.hash_contract.float_quantum,
                    );
                }
                None => {
                    assert_eq!(
                        case.recorded_current.outcome,
                        RecordedOutcomeKind::Candidates
                    );
                    assert!(
                        case.recorded_current.candidate_count > 0,
                        "{}: 候補baselineにも実行失敗baselineにも該当しない",
                        case.id
                    );
                }
            }
            assert!(
                [
                    case.source.kind.as_str(),
                    case.source.title.as_str(),
                    case.source.uri.as_str(),
                    case.source.author.as_str(),
                    case.source.license_spdx.as_str(),
                    case.source.license_uri.as_str(),
                    case.source.attribution.as_str(),
                    case.classification_basis.symmetry_evidence.as_str(),
                    case.classification_basis.position_evidence.as_str(),
                    case.classification_basis.technique_reference.as_str(),
                ]
                .into_iter()
                .all(|field| !field.trim().is_empty()),
                "{}: source/license/classification_basisに空欄がある",
                case.id
            );
        } else {
            assert!(
                case.planned_slot.is_none(),
                "非算入caseが30件slotを予約している: {}",
                case.id
            );
        }
    }
    assert_eq!(materialized_slots.len(), 30);
    assert_eq!(materialized_structures.len(), 30);
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.counts_toward_target)
            .count(),
        30
    );
    let pilot = manifest
        .cases
        .iter()
        .find(|case| case.id == "pilot-head-tail-four-legs")
        .expect("非算入pilotがない");
    assert!(!pilot.counts_toward_target);
    assert!(pilot.recorded_current.execution_failure.is_none());
    assert_eq!(
        pilot.recorded_current.outcome,
        RecordedOutcomeKind::Candidates
    );
    assert_recorded_assessment(
        &pilot.target,
        &pilot.recorded_current,
        &manifest.numeric_policy,
    );
    assert!(pilot.display_name.is_none());
    assert!(fixture_path(&pilot.input.fixture).is_ok());
    assert!(
        [
            pilot.source.kind.as_str(),
            pilot.source.title.as_str(),
            pilot.source.uri.as_str(),
            pilot.source.author.as_str(),
            pilot.source.license_spdx.as_str(),
            pilot.source.license_uri.as_str(),
            pilot.source.attribution.as_str(),
            pilot.classification_basis.symmetry_evidence.as_str(),
            pilot.classification_basis.position_evidence.as_str(),
            pilot.classification_basis.technique_reference.as_str(),
            pilot.recorded_current.time_budget.basis.as_str(),
        ]
        .into_iter()
        .all(|field| !field.trim().is_empty()),
        "pilotの由来・分類・時間根拠に空欄がある"
    );
    assert_eq!(pilot.strata.band, "B");
    assert_eq!(pilot.strata.leaf_count, 6);
    assert_eq!(pilot.strata.symmetry, "symmetric");
    assert_eq!(pilot.strata.position_constraint, "none");
    assert_eq!(pilot.strata.technique_complexity, "simple");
    for hash in [
        pilot.input.fixture_checksum.digest.as_str(),
        pilot.input.structure_hash.as_str(),
        pilot.input.normalized_input_hash.as_str(),
        pilot.recorded_current.normalized_candidate_hash.as_str(),
        pilot.recorded_current.stop_reason_hash.as_str(),
        pilot.recorded_current.normalized_result_hash.as_str(),
    ] {
        assert_eq!(hash.len(), 16, "hashの桁数が違う: {hash}");
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}

#[test]
fn pilot_uses_product_path_and_matches_the_read_only_baseline() {
    let manifest_file = manifest_path();
    let (manifest_before, manifest) = load_manifest().expect("manifestを読めない");
    let case = manifest.cases.first().expect("pilot caseがない");
    let input_file = fixture_path(&case.input.fixture).expect("pilot pathが不正");
    let (input_before, input) = load_input(case).expect("pilot inputを読めない");
    let quantum = manifest.hash_contract.float_quantum;
    let fixture_checksum = fixture_checksum(&input_before).expect("fixture checksum失敗");
    let input_structure_hash = structure_hash(&input.skeleton, quantum).expect("構造hash失敗");
    let normalized_input_hash = normalized_hash(
        &NormalizedInputContract {
            input: &input,
            runner: &manifest.runner_contract,
        },
        quantum,
    )
    .expect("入力hash失敗");
    println!(
        "CORPUS_3A_INPUT fixture_checksum={fixture_checksum} structure_hash={input_structure_hash} normalized_input_hash={normalized_input_hash}"
    );

    let started = Instant::now();
    let run = run_corpus_case(&input, &manifest.runner_contract).expect("pilot実行失敗");
    let elapsed_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let normalized_candidate_hash =
        normalized_hash(&run.candidates, quantum).expect("候補hash失敗");
    let stop_reason_hash = normalized_hash(&run.stop_reasons, quantum).expect("停止理由hash失敗");
    let normalized_result_hash = normalized_hash(
        &DeterminismContract {
            candidates: &run.candidates,
            stop_reasons: &run.stop_reasons,
            metrics: &run.metrics,
        },
        quantum,
    )
    .expect("結果hash失敗");
    println!(
        "CORPUS_3A_RESULT candidate_hash={normalized_candidate_hash} stop_hash={stop_reason_hash} result_hash={normalized_result_hash} elapsed_millis={elapsed_millis} metrics={}",
        serde_json::to_string(&run.metrics).expect("metrics JSON失敗")
    );

    assert_eq!(case.id, "pilot-head-tail-four-legs");
    assert!(case.display_name.is_none());
    assert!(!case.counts_toward_target);
    assert_eq!(case.source.license_spdx, "MIT");
    assert_eq!(case.input.fixture_checksum.algorithm, "fnv1a64");
    assert_eq!(fixture_checksum, case.input.fixture_checksum.digest);
    assert_eq!(input_structure_hash, case.input.structure_hash);
    assert_eq!(normalized_input_hash, case.input.normalized_input_hash);
    assert_eq!(input.skeleton.leaves().len(), case.strata.leaf_count);
    assert_eq!(case.strata.position_constraint, "none");
    assert!(
        input
            .skeleton
            .nodes
            .iter()
            .filter(|node| node.parent.is_some())
            .all(|node| node.tip_pos_2d.is_none()),
        "位置指定なしcaseにtip_pos_2dがある"
    );

    let statuses: Vec<_> = run
        .candidates
        .iter()
        .map(CorpusCandidate::status_tag)
        .collect();
    for (index, metric) in run.metrics.iter().enumerate() {
        let metric = metric
            .as_ref()
            .unwrap_or_else(|| panic!("候補{index}はFoldSessionを作れず安全metricがない"));
        assert_safety_policy(&metric.safety, &manifest.numeric_policy);
        assert_eq!(
            Some(metric.stop_reason.as_str()),
            run.stop_reasons[index].as_deref(),
            "metricと候補順の停止理由が食い違う"
        );
    }
    assert_eq!(run.candidates.len(), case.recorded_current.candidate_count);
    assert_eq!(statuses, case.recorded_current.candidate_statuses);
    assert_eq!(run.stop_reasons, case.recorded_current.stop_reasons);
    let selected =
        selected_candidate(&run.metrics, &manifest.numeric_policy).expect("安全なplanがない");
    assert_eq!(selected, case.recorded_current.selected_candidate_index);
    let metric = run.metrics[selected]
        .as_ref()
        .expect("選択候補にmetricがない");
    assert_eq!(case.target.class, "must_complete");
    assert_expected_class(&case.target.class, metric, &manifest.numeric_policy);
    assert_gaps_near(
        "initial_gaps",
        metric.initial_gaps,
        case.recorded_current.initial_gaps,
        manifest.numeric_policy.gap_abs_tolerance,
    );
    assert_gaps_near(
        "final_gaps",
        metric.final_gaps,
        case.recorded_current.final_gaps,
        manifest.numeric_policy.gap_abs_tolerance,
    );
    assert_near(
        "initial_weighted_gap",
        metric.initial_weighted_gap,
        case.recorded_current.initial_weighted_gap,
        manifest.numeric_policy.weighted_gap_abs_tolerance,
    );
    assert_near(
        "final_weighted_gap",
        metric.final_weighted_gap,
        case.recorded_current.final_weighted_gap,
        manifest.numeric_policy.weighted_gap_abs_tolerance,
    );
    assert_near(
        "improvement_absolute",
        metric.improvement_absolute,
        case.recorded_current.improvement_absolute,
        manifest.numeric_policy.weighted_gap_abs_tolerance,
    );
    // 改善率は`改善量 / 初期score`なので固定の生小数境界にしない。weighted gapの
    // 6.4e-9許容を分子・分母へ1回ずつ伝播させたcase固有の境界を使う。
    let expected_start = case
        .recorded_current
        .initial_weighted_gap
        .abs()
        .max(manifest.numeric_policy.weighted_gap_abs_tolerance);
    let ratio_tolerance = manifest.numeric_policy.weighted_gap_abs_tolerance / expected_start
        + case.recorded_current.improvement_absolute.abs()
            * manifest.numeric_policy.weighted_gap_abs_tolerance
            / expected_start.powi(2);
    assert_near(
        "improvement_ratio",
        metric.improvement_ratio,
        case.recorded_current.improvement_ratio,
        ratio_tolerance,
    );
    assert_safety(
        &metric.safety,
        &case.recorded_current.safety,
        &manifest.numeric_policy,
    );
    assert_eq!(
        normalized_candidate_hash,
        case.recorded_current.normalized_candidate_hash
    );
    assert_eq!(stop_reason_hash, case.recorded_current.stop_reason_hash);
    assert_eq!(
        normalized_result_hash,
        case.recorded_current.normalized_result_hash
    );
    assert_eq!(
        case.recorded_current.time_budget.search_watchdog_millis,
        manifest.runner_contract.search_watchdog_millis
    );
    assert!(
        !case
            .recorded_current
            .time_budget
            .ordinary_test_enforces_elapsed
    );
    // 3-A pilotのdebug実測3回は8,765/8,873/8,808ms。3-Bの30作品には、より重い
    // 折り鶴の27秒超を根拠にしたdebug 90秒、release 10秒、全体300秒を使う。
    assert_eq!(
        case.recorded_current
            .time_budget
            .measured_debug_elapsed_millis,
        Some(8_873)
    );
    assert_eq!(
        case.recorded_current.time_budget.debug_case_limit_millis,
        90_000
    );
    assert_eq!(
        case.recorded_current
            .time_budget
            .measured_release_elapsed_millis,
        None
    );
    assert_eq!(
        case.recorded_current.time_budget.release_case_limit_millis,
        10_000
    );
    assert_eq!(
        case.recorded_current
            .time_budget
            .release_corpus_limit_millis,
        300_000
    );

    let manifest_after = fs::read(&manifest_file).expect("manifest再読込失敗");
    let input_after = fs::read(&input_file).expect("pilot再読込失敗");
    assert_eq!(
        manifest_after, manifest_before,
        "通常検査がmanifestを変えた"
    );
    assert_eq!(input_after, input_before, "通常検査がinput fixtureを変えた");
}

#[test]
fn normalized_hash_ignores_formatting_and_sub_quantum_noise() {
    let first: Value =
        serde_json::from_str(r#"{"angle":90.0,"point":[0.25,-0.5]}"#).expect("hash probe 1");
    let reordered: Value =
        serde_json::from_str(r#"{"point":[0.2500000004,-0.5],"angle":90.0000000004}"#)
            .expect("hash probe 2");
    let changed: Value = serde_json::from_str(r#"{"point":[0.2500000011,-0.5],"angle":90.0}"#)
        .expect("hash probe 3");
    let negative_near_zero: Value = serde_json::from_str(r#"{"coordinate":-0.0000000000000001}"#)
        .expect("hash probe negative zero");
    let positive_near_zero: Value = serde_json::from_str(r#"{"coordinate":0.0000000000000001}"#)
        .expect("hash probe positive zero");
    let first_hash = normalized_hash(&first, FLOAT_TOL).expect("hash probe 1");
    let reordered_hash = normalized_hash(&reordered, FLOAT_TOL).expect("hash probe 2");
    let changed_hash = normalized_hash(&changed, FLOAT_TOL).expect("hash probe 3");
    assert_eq!(first_hash, reordered_hash, "量子化未満の差を区別した");
    assert_ne!(first_hash, changed_hash, "量子化を超える差を隠した");
    assert_eq!(
        normalized_hash(&negative_near_zero, FLOAT_TOL).expect("negative zero hash"),
        normalized_hash(&positive_near_zero, FLOAT_TOL).expect("positive zero hash"),
        "量子化未満の符号付き0を区別した"
    );

    let (_, manifest) = load_manifest().expect("manifestを読めない");
    let case = manifest.cases.first().expect("pilot caseがない");
    let (_, input) = load_input(case).expect("pilot inputを読めない");
    let mut reordered_skeleton = input.skeleton.clone();
    reordered_skeleton.nodes.reverse();
    let mut uniformly_scaled_skeleton = input.skeleton.clone();
    for node in &mut uniformly_scaled_skeleton.nodes {
        if node.parent.is_some() {
            node.length *= 2.0;
        }
    }
    let original_structure_hash = structure_hash(&input.skeleton, FLOAT_TOL).expect("元構造hash");
    assert_eq!(
        original_structure_hash,
        structure_hash(&reordered_skeleton, FLOAT_TOL).expect("並替構造hash"),
        "節点の記述順だけで別作品になった"
    );
    assert_eq!(
        original_structure_hash,
        structure_hash(&uniformly_scaled_skeleton, FLOAT_TOL).expect("一様拡大構造hash"),
        "全長の一様な倍率だけで別作品になった"
    );
}
