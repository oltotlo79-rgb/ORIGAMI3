//! 施策3のcorpus schema、read-only fixture契約、製品相当runner。
//!
//! 3-Aで事前固定した30 slotを3-Bで1件ずつ物質化し、製品と同じcore経路で
//! 完成または安全な改善、停止理由、決定性hashをread-onlyで照合する。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ori3_model::{AlignmentTarget, CreasePattern, Document, FoldStep, Paper};
use ori3_propose::{
    CompletionTolerance, FinishGaps, FinishTarget, FoldGoal, FoldSession, GapWeights, LeafSite,
    Packing, PoseScan, SearchAbort, SearchBudget, SearchCancellation, SearchControl,
    SearchWatchdog, Skeleton, TipSite, VerifiedPlan, body_on_paper, generate, pack,
    search_to_completion, search_to_completion_with_control, verify_search_completion,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MANIFEST_NAME: &str = "manifest.json";

const FUNCTIONAL_SEARCH_CONTRACT: &str = "search_to_completion_no_wall_clock";
const PERFORMANCE_SEARCH_CONTRACT: &str = "search_to_completion_with_control";

#[derive(Clone, Copy)]
enum CorpusExecutionMode {
    FunctionalDeterministic,
    ProductWatchdog,
}

impl CorpusExecutionMode {
    fn contract_tag(self) -> &'static str {
        match self {
            Self::FunctionalDeterministic => "deterministic_no_wall_clock",
            Self::ProductWatchdog => "product_watchdog_30000ms",
        }
    }

    fn product_search_watchdog_millis(self, runner: &RunnerContract) -> Option<u64> {
        match self {
            Self::FunctionalDeterministic => None,
            Self::ProductWatchdog => Some(runner.product_search_watchdog_millis),
        }
    }
}

/// 小数のbaseline照合とhash量子化に使う幅。
///
/// 既存の製品相当10回検査で反復差の実測最大は0、過去のCI座標差は
/// `1.11e-16`だった。採用値`1e-9`はその約900万倍の丸め余裕を持ち、既存の
/// 完成/未完成の最小分離`0.062132...`より7桁以上細い。座標・角・gapはこの幅で
/// 比較し、個数・ID・接続・種類・順序だけを完全一致で比較する。
const FLOAT_TOL: f64 = 1e-9;

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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
    // 正本再生成前のnormalized input hashを保つため、旧manifestではこの2項目を
    // 省略可能にする。新schemaへ確定するときに必須fieldへ切り替える。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    functional_search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    performance_search: Option<String>,
    // 製品性能だけに適用する壁時計上限だとJSON上でも明示する。旧schema v2の
    // search_watchdog_millisは読み取りaliasとしてだけ残す。
    #[serde(
        rename = "product_search_watchdog_millis",
        alias = "search_watchdog_millis"
    )]
    product_search_watchdog_millis: u64,
    gap_weights: GapMetric,
    completion_tolerance: GapMetric,
}

impl RunnerContract {
    fn functional_search(&self) -> &str {
        self.functional_search
            .as_deref()
            .unwrap_or(FUNCTIONAL_SEARCH_CONTRACT)
    }

    fn performance_search(&self) -> &str {
        self.performance_search
            .as_deref()
            .unwrap_or(PERFORMANCE_SEARCH_CONTRACT)
    }
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Repetitions {
    determinism: usize,
    performance_release: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseAggregation {
    completion: String,
    partial: String,
    safety_scope: String,
    no_plan: String,
}

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicMetric {
    name: String,
    fields: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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
    #[serde(
        rename = "product_search_watchdog_millis",
        alias = "search_watchdog_millis"
    )]
    product_search_watchdog_millis: u64,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    functional_runner: NormalizedFunctionalRunnerContract<'a>,
}

#[derive(Serialize)]
struct NormalizedFunctionalRunnerContract<'a> {
    product_path: &'a str,
    paper_normalization: &'a str,
    pack_starts: usize,
    packing_max_candidates: usize,
    candidate_execution: &'a str,
    generation_failure: &'a str,
    fold_session_failure: &'a str,
    search_abort: &'a str,
    with_fold_plan: bool,
    search_budget: &'a SearchBudgetContract,
    verification_scan_steps: usize,
    rebuild_scan_steps: usize,
    functional_search: &'a str,
    gap_weights: GapMetric,
    completion_tolerance: GapMetric,
}

impl<'a> NormalizedInputContract<'a> {
    fn new(input: &'a CorpusInput, runner: &'a RunnerContract) -> Self {
        Self {
            input,
            functional_runner: NormalizedFunctionalRunnerContract {
                product_path: &runner.product_path,
                paper_normalization: &runner.paper_normalization,
                pack_starts: runner.pack_starts,
                packing_max_candidates: runner.packing_max_candidates,
                candidate_execution: &runner.candidate_execution,
                generation_failure: &runner.generation_failure,
                fold_session_failure: &runner.fold_session_failure,
                search_abort: &runner.search_abort,
                with_fold_plan: runner.with_fold_plan,
                search_budget: &runner.search_budget,
                verification_scan_steps: runner.verification_scan_steps,
                rebuild_scan_steps: runner.rebuild_scan_steps,
                functional_search: runner.functional_search(),
                gap_weights: runner.gap_weights,
                completion_tolerance: runner.completion_tolerance,
            },
        }
    }
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

fn product_search_control<'a>(
    runner: &RunnerContract,
    cancellation: &'a dyn SearchCancellation,
) -> SearchControl<'a> {
    SearchControl::new(
        SearchWatchdog {
            max_millis: runner.product_search_watchdog_millis,
        },
        cancellation,
    )
}

fn calculate_candidate(
    input: &CorpusInput,
    runner: &RunnerContract,
    execution_mode: CorpusExecutionMode,
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
    let search_started = Instant::now();
    let outcome = match execution_mode {
        CorpusExecutionMode::FunctionalDeterministic => {
            search_to_completion(&session, &goal, weights, budget, tolerance)
        }
        CorpusExecutionMode::ProductWatchdog => {
            let never_cancelled = || false;
            let control = product_search_control(runner, &never_cancelled);
            search_to_completion_with_control(&session, &goal, weights, budget, tolerance, &control)
                .map_err(|abort| CandidateRunError::SearchAborted(abort.into()))?
        }
    };
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
    run_corpus_case_with_mode(input, runner, CorpusExecutionMode::FunctionalDeterministic)
}

fn run_corpus_case_with_mode(
    input: &CorpusInput,
    runner: &RunnerContract,
    execution_mode: CorpusExecutionMode,
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
                    match calculate_candidate(
                        input,
                        runner,
                        execution_mode,
                        packing,
                        paper_w,
                        paper_h,
                    ) {
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

// 旧暫定値は性能metadataの厳密契約として保持するが、壁時計なしの機能solveには
// 適用しない。製品探索の実上限は別契約の30,000msであり、これらはenforced=false。
const MAX_DEBUG_CASE_MILLIS: u64 = 90_000;
const MAX_RELEASE_CASE_MILLIS: u64 = 10_000;
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

/// 3-Cの決定性比較に使うoutcome全体の指紋。
///
/// 候補を返したrunと30秒watchdogで提案全体が失敗したrunを同一視しない。
/// 小数は既存の1e-9量子化hashで比較し、候補・停止理由・手順の配列順は
/// canonical JSON内でそのまま保持する。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum Stage3cFingerprint {
    Candidates {
        normalized_candidate_hash: String,
        stop_reason_hash: String,
        normalized_result_hash: String,
    },
    ExecutionFailure {
        phase: String,
        reason: AbortKind,
        normalized_failure_hash: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct Stage3cRunRecord {
    phase: String,
    repetition: usize,
    case_index: usize,
    case_id: String,
    planned_slot: String,
    band: String,
    leaf_count: usize,
    elapsed_millis: u64,
    matches_first_in_phase: bool,
    matches_recorded_current: bool,
    fingerprint: Stage3cFingerprint,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Stage3cRoundRecord {
    phase: String,
    repetition: usize,
    case_count: usize,
    #[serde(
        rename = "phase_elapsed_millis",
        alias = "product_elapsed_millis"
    )]
    phase_elapsed_millis: u64,
    collector_wall_elapsed_millis: u64,
}

#[derive(Debug)]
struct Stage3cPreparedCase {
    case_id: String,
    planned_slot: String,
    band: String,
    leaf_count: usize,
    input_file: PathBuf,
    input_bytes: Vec<u8>,
    input: CorpusInput,
    fixture_checksum: String,
    structure_hash: String,
    normalized_input_hash: String,
    recorded_fingerprint: Stage3cFingerprint,
}

#[derive(Debug, Deserialize, Serialize)]
struct Stage3cCollectionSummary {
    determinism_run_count: usize,
    determinism_mismatches_from_first: usize,
    determinism_mismatches_from_recorded_current: usize,
    performance_run_count: usize,
    performance_mismatches_from_first: usize,
    performance_mismatches_from_recorded_current: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct Stage3cCollectionEnvironment {
    started_unix_millis: u64,
    machine: Stage3cMetricsMachine,
}

#[derive(Debug, Deserialize, Serialize)]
struct Stage3cCollectionFinished {
    finished_unix_millis: u64,
}

#[derive(Debug)]
struct Stage3cCollectionStart {
    phase: String,
    cases: usize,
    determinism_repetitions: usize,
    performance_repetitions: usize,
    profile: String,
    functional_search: String,
    performance_search: String,
    product_watchdog_millis: u64,
    require_recorded_current: bool,
    target_met_is_not_implied: bool,
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
    observe_case_with_mode(
        input_bytes,
        input,
        manifest,
        CorpusExecutionMode::FunctionalDeterministic,
    )
}

fn observe_case_with_mode(
    input_bytes: &[u8],
    input: &CorpusInput,
    manifest: &CorpusManifest,
    execution_mode: CorpusExecutionMode,
) -> Result<ObservedCase, String> {
    let quantum = manifest.hash_contract.float_quantum;
    let fixture_checksum = fixture_checksum(input_bytes)?;
    let input_structure_hash = structure_hash(&input.skeleton, quantum)?;
    let normalized_input_hash = normalized_hash(
        &NormalizedInputContract::new(input, &manifest.runner_contract),
        quantum,
    )?;
    let started = Instant::now();
    let run = run_corpus_case_with_mode(input, &manifest.runner_contract, execution_mode);
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

fn observed_fingerprint(outcome: &ObservedOutcome) -> Stage3cFingerprint {
    match outcome {
        ObservedOutcome::Candidates(observed) => Stage3cFingerprint::Candidates {
            normalized_candidate_hash: observed.normalized_candidate_hash.clone(),
            stop_reason_hash: observed.stop_reason_hash.clone(),
            normalized_result_hash: observed.normalized_result_hash.clone(),
        },
        ObservedOutcome::ExecutionFailure(observed) => Stage3cFingerprint::ExecutionFailure {
            phase: observed.contract.phase.clone(),
            reason: observed.contract.reason,
            normalized_failure_hash: observed.normalized_failure_hash.clone(),
        },
    }
}

fn recorded_fingerprint(current: &RecordedCurrent) -> Stage3cFingerprint {
    match current.outcome {
        RecordedOutcomeKind::Candidates => {
            assert!(
                current.execution_failure.is_none(),
                "候補baselineにexecution_failureがある"
            );
            Stage3cFingerprint::Candidates {
                normalized_candidate_hash: current.normalized_candidate_hash.clone(),
                stop_reason_hash: current.stop_reason_hash.clone(),
                normalized_result_hash: current.normalized_result_hash.clone(),
            }
        }
        RecordedOutcomeKind::ExecutionFailure => {
            let failure = current
                .execution_failure
                .as_ref()
                .expect("実行失敗baselineにfailure契約がない");
            Stage3cFingerprint::ExecutionFailure {
                phase: failure.phase.clone(),
                reason: failure.reason,
                normalized_failure_hash: failure.normalized_failure_hash.clone(),
            }
        }
    }
}

fn print_stage_3c_json<T: Serialize>(prefix: &str, value: &T) {
    println!(
        "{prefix}={}",
        serde_json::to_string(value).expect("3-C JSONを直列化できない")
    );
    io::stdout().flush().expect("3-C進捗をflushできない");
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
    completed: bool,
    improvement_ratio: Option<f64>,
}

fn assert_product_time_metadata_contract(
    case_id: &str,
    expectation: &RecordedCurrent,
    runner: &RunnerContract,
) {
    assert_eq!(
        expectation.time_budget.product_search_watchdog_millis,
        runner.product_search_watchdog_millis
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
    assert_recorded_assessment(
        &case.target,
        &case.recorded_current,
        &manifest.numeric_policy,
    );
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
                    "CORPUS_CASE id={case_id} functional_elapsed_millis={} baseline=matched outcome=execution_failure phase={} reason={:?} failure_hash={} acceptance=false",
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
                    "CORPUS_CASE id={case_id} functional_elapsed_millis={} baseline=matched outcome=candidates candidate_hash={} stop_hash={} result_hash={} class_pass={} safety_pass={}",
                    observed.elapsed_millis,
                    actual.normalized_candidate_hash,
                    actual.stop_reason_hash,
                    actual.normalized_result_hash,
                    class_pass,
                    safety_pass,
                );
                CaseEvaluation {
                    case_id: case_id.to_owned(),
                    expectation_class: case.target.class.clone(),
                    class_pass,
                    safety_pass,
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

fn collect_stage_3c_phase(
    phase: &str,
    repetitions: usize,
    prepared_cases: &[Stage3cPreparedCase],
    manifest: &CorpusManifest,
    execution_mode: CorpusExecutionMode,
) -> (usize, usize, usize) {
    let mut first_fingerprints = vec![None; prepared_cases.len()];
    let mut run_count = 0;
    let mut mismatches_from_first = 0;
    let mut mismatches_from_recorded_current = 0;
    for repetition in 1..=repetitions {
        let round_started = Instant::now();
        let mut phase_elapsed_millis = 0_u64;
        for (case_index, case) in prepared_cases.iter().enumerate() {
            let observed =
                observe_case_with_mode(&case.input_bytes, &case.input, manifest, execution_mode)
                    .unwrap_or_else(|error| panic!("{}: 3-C実行失敗: {error}", case.case_id));
            phase_elapsed_millis = phase_elapsed_millis.saturating_add(observed.elapsed_millis);
            assert_eq!(observed.fixture_checksum, case.fixture_checksum);
            assert_eq!(observed.structure_hash, case.structure_hash);
            assert_eq!(observed.normalized_input_hash, case.normalized_input_hash);
            let fingerprint = observed_fingerprint(&observed.outcome);
            let matches_first_in_phase = first_fingerprints[case_index]
                .as_ref()
                .is_none_or(|first| first == &fingerprint);
            if first_fingerprints[case_index].is_none() {
                first_fingerprints[case_index] = Some(fingerprint.clone());
            } else if !matches_first_in_phase {
                mismatches_from_first += 1;
            }
            let matches_recorded_current = fingerprint == case.recorded_fingerprint;
            if !matches_recorded_current {
                mismatches_from_recorded_current += 1;
            }
            run_count += 1;
            print_stage_3c_json(
                "CORPUS_3C_RUN_JSON",
                &Stage3cRunRecord {
                    phase: phase.to_owned(),
                    repetition,
                    case_index: case_index + 1,
                    case_id: case.case_id.clone(),
                    planned_slot: case.planned_slot.clone(),
                    band: case.band.clone(),
                    leaf_count: case.leaf_count,
                    elapsed_millis: observed.elapsed_millis,
                    matches_first_in_phase,
                    matches_recorded_current,
                    fingerprint,
                },
            );
        }
        print_stage_3c_json(
            "CORPUS_3C_ROUND_JSON",
            &Stage3cRoundRecord {
                phase: phase.to_owned(),
                repetition,
                case_count: prepared_cases.len(),
                phase_elapsed_millis,
                collector_wall_elapsed_millis: u64::try_from(round_started.elapsed().as_millis())
                    .unwrap_or(u64::MAX),
            },
        );
    }
    (
        run_count,
        mismatches_from_first,
        mismatches_from_recorded_current,
    )
}

/// 3-Cの数値収集専用入口。失敗標本を通常検査から外す`ignore`ではない。
///
/// `--release --ignored --exact`で明示したときだけ、同じ30作品・同じseedを、
/// 壁時計なしの機能runnerで10回、別系列の30,000ms製品runnerで5回実行する。
/// stdoutへ固定JSON行を出すだけで、manifestやfixtureは書き換えない。
#[test]
#[ignore = "3-Cの30×10決定性・30×5 release性能の明示収集専用"]
fn collect_stage_3c_release_measurements() {
    if cfg!(debug_assertions) {
        panic!("3-C collectorは--releaseでだけ実行する");
    }
    let _guard = corpus_run_guard();
    let manifest_file = manifest_path();
    let (manifest_before, manifest) = load_manifest().expect("manifestを読めない");
    assert_eq!(manifest.repetitions.determinism, 10);
    assert_eq!(manifest.repetitions.performance_release, 5);
    let requested_phase = std::env::var("ORI3_CORPUS_3C_PHASE")
        .expect("ORI3_CORPUS_3C_PHASEをfunctional_determinismまたはproduct_performanceにする");
    let require_recorded_current = match std::env::var("ORI3_CORPUS_REQUIRE_RECORDED_CURRENT") {
        Ok(value) => {
            assert_eq!(
                value, "1",
                "ORI3_CORPUS_REQUIRE_RECORDED_CURRENTは1を明示するか、未設定にする"
            );
            true
        }
        Err(std::env::VarError::NotPresent) => false,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("ORI3_CORPUS_REQUIRE_RECORDED_CURRENTはUnicodeの1を明示する")
        }
    };
    assert!(
        matches!(
            requested_phase.as_str(),
            "functional_determinism" | "product_performance"
        ),
        "ORI3_CORPUS_3C_PHASEはfunctional_determinismまたはproduct_performanceにする"
    );
    assert!(
        requested_phase == "functional_determinism" || !require_recorded_current,
        "recorded_current照合は機能決定性phaseだけに適用する"
    );

    let prepared_cases: Vec<_> = manifest
        .planned_slots
        .iter()
        .map(|slot| {
            let case = manifest
                .cases
                .iter()
                .find(|case| case.id == slot.case_id)
                .unwrap_or_else(|| panic!("manifestにcaseがない: {}", slot.case_id));
            assert!(case.counts_toward_target);
            let input_file = fixture_path(&case.input.fixture).expect("corpus input pathが不正");
            let (input_bytes, input) = load_input(case).expect("corpus inputを読めない");
            let checksum = fixture_checksum(&input_bytes).expect("fixture checksum失敗");
            let input_structure_hash =
                structure_hash(&input.skeleton, manifest.hash_contract.float_quantum)
                    .expect("structure hash失敗");
            let input_hash = normalized_hash(
                &NormalizedInputContract::new(&input, &manifest.runner_contract),
                manifest.hash_contract.float_quantum,
            )
            .expect("normalized input hash失敗");
            assert_eq!(checksum, case.input.fixture_checksum.digest);
            assert_eq!(input_structure_hash, case.input.structure_hash);
            assert_eq!(input_hash, case.input.normalized_input_hash);
            Stage3cPreparedCase {
                case_id: case.id.clone(),
                planned_slot: slot.slot.clone(),
                band: case.strata.band.clone(),
                leaf_count: case.strata.leaf_count,
                input_file,
                input_bytes,
                input,
                fixture_checksum: checksum,
                structure_hash: input_structure_hash,
                normalized_input_hash: input_hash,
                recorded_fingerprint: recorded_fingerprint(&case.recorded_current),
            }
        })
        .collect();
    assert_eq!(prepared_cases.len(), 30);

    print_stage_3c_json(
        "CORPUS_3C_ENVIRONMENT_JSON",
        &Stage3cCollectionEnvironment {
            started_unix_millis: system_time_millis(SystemTime::now()),
            machine: stage_3c_machine_evidence(),
        },
    );
    println!(
        "CORPUS_3C_COLLECTION_START phase={} cases=30 determinism_repetitions={} performance_repetitions={} profile=release functional_search={} performance_search={} product_watchdog_millis={} require_recorded_current={} target_met_is_not_implied=true",
        requested_phase,
        manifest.repetitions.determinism,
        manifest.repetitions.performance_release,
        manifest.runner_contract.functional_search(),
        manifest.runner_contract.performance_search(),
        manifest.runner_contract.product_search_watchdog_millis,
        require_recorded_current,
    );
    io::stdout().flush().expect("3-C開始行をflushできない");

    let collection = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let determinism = if requested_phase == "functional_determinism" {
            collect_stage_3c_phase(
                "functional_determinism",
                manifest.repetitions.determinism,
                &prepared_cases,
                &manifest,
                CorpusExecutionMode::FunctionalDeterministic,
            )
        } else {
            (0, 0, 0)
        };
        assert_eq!(
            determinism.1, 0,
            "壁時計なし機能runnerの10周で系列内fingerprintが一致しない"
        );
        if require_recorded_current {
            assert_eq!(
                determinism.2, 0,
                "壁時計なし機能runnerの10周がrecorded_currentと一致しない"
            );
        }
        let performance = if requested_phase == "product_performance" {
            collect_stage_3c_phase(
                "product_performance",
                manifest.repetitions.performance_release,
                &prepared_cases,
                &manifest,
                CorpusExecutionMode::ProductWatchdog,
            )
        } else {
            (0, 0, 0)
        };
        Stage3cCollectionSummary {
            determinism_run_count: determinism.0,
            determinism_mismatches_from_first: determinism.1,
            determinism_mismatches_from_recorded_current: determinism.2,
            performance_run_count: performance.0,
            performance_mismatches_from_first: performance.1,
            performance_mismatches_from_recorded_current: performance.2,
        }
    }));

    let manifest_after = fs::read(&manifest_file).expect("manifest再読込失敗");
    assert_eq!(
        manifest_after, manifest_before,
        "3-C collectorがmanifestを変えた"
    );
    for case in &prepared_cases {
        let input_after = fs::read(&case.input_file).expect("corpus input再読込失敗");
        assert_eq!(
            input_after, case.input_bytes,
            "3-C collectorが{}のinput fixtureを変えた",
            case.case_id
        );
    }
    match collection {
        Ok(summary) => print_stage_3c_json("CORPUS_3C_SUMMARY_JSON", &summary),
        Err(payload) => std::panic::resume_unwind(payload),
    }
    print_stage_3c_json(
        "CORPUS_3C_FINISHED_JSON",
        &Stage3cCollectionFinished {
            finished_unix_millis: system_time_millis(SystemTime::now()),
        },
    );
}

/// 3-Bで1件ずつbaselineを確定する専用入口。通常検査からは呼ばない。
///
/// `ORI3_CORPUS_CASE`で明示した1件だけを壁時計なしの機能runnerへ通し、正本候補を
/// JSONへ出す。機能測定の経過時間を製品性能値へ混ぜず、既存の`time_budget`は保つ。
/// fixtureへは書かない。移行中は旧測定commandとの互換のため
/// `ORI3_CORPUS_TIME_FREE=1`も受け付けるが、未設定時と同じ機能runnerになる。
#[test]
#[ignore = "3-B baselineの明示的な再生成専用"]
fn regenerate_one_corpus_baseline() {
    let _guard = corpus_run_guard();
    if std::env::var("ORI3_REGENERATE_CORPUS_FROM_EVIDENCE").as_deref() == Ok("1") {
        assert!(
            !cfg!(debug_assertions),
            "corpus正本再生成はrelease evidenceでだけ実行する"
        );
        regenerate_corpus_from_evidence();
        return;
    }
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
            &NormalizedInputContract::new(&input, &manifest.runner_contract),
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
    let execution_mode = match std::env::var("ORI3_CORPUS_TIME_FREE") {
        Ok(value) => {
            assert_eq!(
                value, "1",
                "ORI3_CORPUS_TIME_FREEは1を明示するか、未設定にする"
            );
            CorpusExecutionMode::FunctionalDeterministic
        }
        Err(std::env::VarError::NotPresent) => CorpusExecutionMode::FunctionalDeterministic,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("ORI3_CORPUS_TIME_FREEはUnicodeの1を明示する")
        }
    };
    let actual_search_watchdog_millis = execution_mode
        .product_search_watchdog_millis(&manifest.runner_contract)
        .map_or_else(|| "none".to_owned(), |value| value.to_string());
    let observed = observe_case_with_mode(&input_bytes, &input, &manifest, execution_mode)
        .expect("corpus case実行失敗");
    let expected_class = manifest.cases[case_index].target.class.clone();
    let fixture_checksum = observed.fixture_checksum;
    let input_structure_hash = observed.structure_hash;
    let normalized_input_hash = observed.normalized_input_hash;
    let elapsed_millis = observed.elapsed_millis;
    let case = &mut manifest.cases[case_index];
    case.input.fixture_checksum.algorithm = "fnv1a64".to_owned();
    case.input.fixture_checksum.digest = fixture_checksum;
    case.input.structure_hash = input_structure_hash;
    case.input.normalized_input_hash = normalized_input_hash;
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
                "CORPUS_3B_TARGET_STATUS case={} class={} selected_status={} all_returned_plans_safe={} target_met={} functional_elapsed_millis={} mode={} actual_search_watchdog_millis={}",
                case_id,
                expected_class,
                metric.status,
                all_returned_plans_safe,
                expectation_met,
                elapsed_millis,
                execution_mode.contract_tag(),
                actual_search_watchdog_millis,
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
                "CORPUS_3B_TARGET_STATUS case={} class={} outcome=execution_failure target_met=false functional_elapsed_millis={} mode={} actual_search_watchdog_millis={}",
                case_id,
                expected_class,
                elapsed_millis,
                execution_mode.contract_tag(),
                actual_search_watchdog_millis,
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

/// 通常のdebug検査では静的契約だけを確かめ、release jobだけで全30件を実行する。
///
/// 折り鶴のdebug初回実測は27秒超で、壁時計なしの全30件を通常debug jobには置けない。
/// releaseでは保存した機能現在値だけを照合する。性能は別のProductWatchdog経路で測る。
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
    if cfg!(debug_assertions) {
        println!(
            "CORPUS_FUNCTIONAL_LIVE skipped_in_debug=true cases=30 release_exact_required=true"
        );
        return;
    }
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
    // この検査のgreenは「記録済み機能現在値の再現」だけを表す。目標未達は上の
    // 独立集計に必ず残す。性能はProductWatchdogの5周証拠だけで扱う。
    assert!(
        baseline_mismatches.is_empty(),
        "recorded-current mismatch cases: {baseline_mismatches:?}"
    );
}

#[test]
fn functional_and_product_runners_keep_their_separate_time_contracts() {
    let (_, manifest) = load_manifest().expect("manifestを読めない");
    let runner = &manifest.runner_contract;
    assert_eq!(runner.functional_search(), FUNCTIONAL_SEARCH_CONTRACT);
    assert_eq!(runner.performance_search(), PERFORMANCE_SEARCH_CONTRACT);
    assert_eq!(
        runner.functional_search.as_deref(),
        Some(FUNCTIONAL_SEARCH_CONTRACT)
    );
    assert_eq!(
        runner.performance_search.as_deref(),
        Some(PERFORMANCE_SEARCH_CONTRACT)
    );
    assert_eq!(runner.product_search_watchdog_millis, 30_000);
    assert_eq!(
        CorpusExecutionMode::FunctionalDeterministic.product_search_watchdog_millis(runner),
        None
    );
    assert_eq!(
        CorpusExecutionMode::ProductWatchdog.product_search_watchdog_millis(runner),
        Some(30_000)
    );

    let never_cancelled = || false;
    let product_control = product_search_control(runner, &never_cancelled);
    assert_eq!(product_control.watchdog().max_millis, 30_000);
    for case in manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
    {
        assert_product_time_metadata_contract(&case.id, &case.recorded_current, runner);
    }
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
        0
    );
    assert_eq!(manifest.hash_contract.algorithm, "fnv1a64");
    assert_eq!(manifest.hash_contract.digest_encoding, "lowercase-hex-16");
    assert_eq!(
        manifest.hash_contract.fixture_checksum_scope,
        "utf8-text-crlf-normalized-to-lf"
    );
    assert_eq!(
        manifest.hash_contract.input_normalization,
        "typed-input-plus-functional-runner-contract-canonical-json-v2"
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
    assert_eq!(
        manifest.runner_contract.functional_search(),
        FUNCTIONAL_SEARCH_CONTRACT
    );
    assert_eq!(
        manifest.runner_contract.performance_search(),
        PERFORMANCE_SEARCH_CONTRACT
    );
    assert_eq!(
        manifest.runner_contract.functional_search.as_deref(),
        Some(FUNCTIONAL_SEARCH_CONTRACT)
    );
    assert_eq!(
        manifest.runner_contract.performance_search.as_deref(),
        Some(PERFORMANCE_SEARCH_CONTRACT)
    );
    assert_eq!(
        manifest.runner_contract.product_search_watchdog_millis,
        30_000
    );
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
    assert!(
        manifest
            .classification_contract
            .symmetry
            .contains("rooted-tree")
    );
    assert!(manifest.classification_contract.simple.contains("ordinary"));
    assert!(
        manifest
            .classification_contract
            .compound
            .contains("compound")
    );
    assert!(
        manifest
            .case_aggregation
            .completion
            .contains("any-candidate")
    );
    assert!(
        manifest
            .case_aggregation
            .partial
            .contains("lowest-final-weighted-gap")
    );
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
    assert!(
        manifest
            .planned_slots
            .iter()
            .filter(|slot| !slot.anchor)
            .all(|slot| slot.case_id.starts_with("leaves-"))
    );
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
                    &NormalizedInputContract::new(&input, &manifest.runner_contract),
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
        &NormalizedInputContract::new(&input, &manifest.runner_contract),
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
        case.recorded_current
            .time_budget
            .product_search_watchdog_millis,
        manifest.runner_contract.product_search_watchdog_millis
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

// Stage 3-C stores measurements in a separate read-only fixture. The values
// reproduce recorded_current; they do not claim that target classes are met.
const STAGE_3C_METRICS_NAME: &str = "stage-3c-release-metrics.json";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsFixture {
    schema_version: u32,
    corpus_id: String,
    stage: String,
    // 機能10周とProduct性能5周の両証拠を包含する収集期間。性能統計そのものには
    // Product phaseの壁時計値だけを使う。
    measurement_started_unix_millis: u64,
    measurement_finished_unix_millis: u64,
    runner: Stage3cMetricsRunner,
    fixture_integrity: Stage3cMetricsFixtureIntegrity,
    // Product phaseを実測したmachine。機能fingerprintはmachine非依存契約である。
    machine: Stage3cMetricsMachine,
    cases: Vec<Stage3cMetricsCase>,
    performance: Stage3cMetricsPerformance,
    outliers: Stage3cMetricsOutliers,
    gate_proposal: Stage3cMetricsGateProposal,
    target_summary: Stage3cMetricsTargetSummary,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsRunner {
    profile: String,
    determinism_repetitions: usize,
    performance_repetitions: usize,
    functional_search: String,
    performance_search: String,
    product_search_watchdog_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsFixtureIntegrity {
    algorithm: String,
    manifest_checksum: String,
    inputs: Vec<Stage3cMetricsInputChecksum>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsInputChecksum {
    case_id: String,
    checksum: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsMachine {
    operating_system: Option<String>,
    architecture: Option<String>,
    logical_cpu_count: Option<usize>,
    physical_core_count: Option<usize>,
    cpu_model: Option<String>,
    physical_memory_bytes: Option<u64>,
    rust_version: Option<String>,
    rust_host: Option<String>,
    cargo_version: Option<String>,
    target_dir: Option<String>,
    profile: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsCase {
    case_id: String,
    planned_slot: String,
    band: String,
    leaf_count: usize,
    recorded_current_fingerprint: Stage3cMetricsFingerprint,
    determinism: Stage3cMetricsPhase,
    performance: Stage3cMetricsPerformancePhase,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsPhase {
    runs: Vec<Stage3cMetricsRun>,
    mismatches_from_first: usize,
    mismatches_from_recorded_current: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsRun {
    repetition: usize,
    matches_first: bool,
    matches_recorded_current: bool,
    fingerprint: Stage3cMetricsFingerprint,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsPerformancePhase {
    runs: Vec<Stage3cMetricsPerformanceRun>,
    candidate_outcomes: usize,
    watchdog_expired_outcomes: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsPerformanceRun {
    repetition: usize,
    elapsed_millis: u64,
    observed_outcome: Stage3cMetricsPerformanceOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum Stage3cMetricsPerformanceOutcome {
    Candidates,
    ExecutionFailure { phase: String, reason: AbortKind },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum Stage3cMetricsFingerprint {
    Candidates {
        normalized_candidate_hash: String,
        stop_reason_hash: String,
        normalized_result_hash: String,
    },
    ExecutionFailure {
        phase: String,
        reason: AbortKind,
        normalized_failure_hash: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsElapsed {
    case_id: String,
    repetition: usize,
    elapsed_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsBand {
    band: String,
    values: Vec<Stage3cMetricsElapsed>,
    median_millis: f64,
    p95_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsCorpusRound {
    repetition: usize,
    product_elapsed_millis: u64,
    collector_wall_elapsed_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsPerformance {
    all_values: Vec<Stage3cMetricsElapsed>,
    all_median_millis: f64,
    all_p95_millis: u64,
    bands: Vec<Stage3cMetricsBand>,
    corpus_rounds: Vec<Stage3cMetricsCorpusRound>,
    corpus_median_millis: f64,
    corpus_p95_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsOutliers {
    method: String,
    scope: String,
    excluded_from_aggregates: bool,
    values: Vec<Stage3cMetricsOutlier>,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsOutlier {
    case_id: String,
    repetition: usize,
    elapsed_millis: u64,
    lower_fence_millis: f64,
    upper_fence_millis: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsGateProposal {
    source: String,
    performance_baseline_fraction: f64,
    reference_p95_millis: u64,
    raw_gate_millis: u64,
    proposed_gate_seconds: u64,
    proposed_gate_millis: u64,
    case_source: String,
    case_reference_p95_millis: u64,
    case_raw_gate_millis: u64,
    proposed_case_gate_seconds: u64,
    proposed_case_gate_millis: u64,
    enforced: bool,
    status: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stage3cMetricsTargetSummary {
    basis: String,
    target_met: usize,
    target_unmet: usize,
    must_complete_met: usize,
    must_complete_total: usize,
    safe_partial_met: usize,
    safe_partial_total: usize,
}

fn stage_3c_metrics_path() -> PathBuf {
    corpus_fixture_root().join(STAGE_3C_METRICS_NAME)
}

fn stage_3c_metrics_recorded_fingerprint(current: &RecordedCurrent) -> Stage3cMetricsFingerprint {
    match current.outcome {
        RecordedOutcomeKind::Candidates => {
            assert!(current.execution_failure.is_none());
            Stage3cMetricsFingerprint::Candidates {
                normalized_candidate_hash: current.normalized_candidate_hash.clone(),
                stop_reason_hash: current.stop_reason_hash.clone(),
                normalized_result_hash: current.normalized_result_hash.clone(),
            }
        }
        RecordedOutcomeKind::ExecutionFailure => {
            let failure = current
                .execution_failure
                .as_ref()
                .expect("recorded execution failure requires its contract");
            Stage3cMetricsFingerprint::ExecutionFailure {
                phase: failure.phase.clone(),
                reason: failure.reason,
                normalized_failure_hash: failure.normalized_failure_hash.clone(),
            }
        }
    }
}

fn stage_3c_median_millis(values: &[u64]) -> f64 {
    assert!(!values.is_empty(), "median requires one or more values");
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if !sorted.len().is_multiple_of(2) {
        sorted[middle] as f64
    } else {
        (sorted[middle - 1] as f64 + sorted[middle] as f64) / 2.0
    }
}

// elapsedの入力分解能は整数1ms。中央値とTukey fenceだけが除算で小数になるため、
// その100万分の1である1e-6msを再計算照合の許容差にする。1msの観測境界や
// P95順位を動かせない幅で、計算小数を厳密一致しないためだけの余裕である。
const STAGE_3C_DERIVED_MILLIS_TOLERANCE: f64 = 1e-6;

fn stage_3c_nearest_rank(values: &[u64], numerator: usize, denominator: usize) -> u64 {
    assert!(!values.is_empty(), "percentile requires one or more values");
    assert!(numerator > 0 && numerator <= denominator);
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = sorted.len().saturating_mul(numerator).div_ceil(denominator);
    sorted[rank.saturating_sub(1)]
}

fn stage_3c_gate_from_p95(p95_millis: u64) -> (u64, u64, u64) {
    // Keep the measured P95 and this 80%-of-gate proposal distinct. The
    // proposal is ceil(P95 / 0.8), followed by a whole-second ceiling.
    let raw_gate_millis = p95_millis.saturating_mul(5).div_ceil(4);
    let proposed_gate_seconds = raw_gate_millis.div_ceil(1_000);
    let proposed_gate_millis = proposed_gate_seconds.saturating_mul(1_000);
    (raw_gate_millis, proposed_gate_seconds, proposed_gate_millis)
}

fn required_evidence_path(name: &str) -> PathBuf {
    let value = std::env::var(name).unwrap_or_else(|_| panic!("{name}でevidence logを指定する"));
    let path = PathBuf::from(value);
    assert!(path.is_file(), "{name}がfileではない: {}", path.display());
    path
}

fn prefixed_json_records<T>(path: &Path, prefix: &str) -> Vec<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}を読めない: {error}", path.display()));
    text.lines()
        .filter_map(|line| line.strip_prefix(prefix))
        .map(|json| {
            serde_json::from_str(json)
                .unwrap_or_else(|error| panic!("{}の{prefix} JSONが不正: {error}", path.display()))
        })
        .collect()
}

fn exactly_one_prefixed_json<T>(path: &Path, prefix: &str) -> T
where
    T: for<'de> Deserialize<'de>,
{
    let mut records = prefixed_json_records(path, prefix);
    assert_eq!(
        records.len(),
        1,
        "{}には{prefix}がexact 1件必要",
        path.display()
    );
    records.pop().expect("exact 1件を確認済み")
}

fn optional_one_prefixed_json<T>(path: &Path, prefix: &str) -> Option<T>
where
    T: for<'de> Deserialize<'de>,
{
    let mut records = prefixed_json_records(path, prefix);
    assert!(
        records.len() <= 1,
        "{}には{prefix}を最大1件だけ置ける",
        path.display()
    );
    records.pop()
}

fn take_collection_start_field(
    fields: &mut BTreeMap<String, String>,
    name: &str,
) -> String {
    fields
        .remove(name)
        .unwrap_or_else(|| panic!("COLLECTION_STARTに{name}がない"))
}

fn parse_collection_start_usize(fields: &mut BTreeMap<String, String>, name: &str) -> usize {
    let value = take_collection_start_field(fields, name);
    value
        .parse()
        .unwrap_or_else(|error| panic!("COLLECTION_STARTの{name}={value}がusizeでない: {error}"))
}

fn parse_collection_start_u64(fields: &mut BTreeMap<String, String>, name: &str) -> u64 {
    let value = take_collection_start_field(fields, name);
    value
        .parse()
        .unwrap_or_else(|error| panic!("COLLECTION_STARTの{name}={value}がu64でない: {error}"))
}

fn parse_collection_start_bool(fields: &mut BTreeMap<String, String>, name: &str) -> bool {
    match take_collection_start_field(fields, name).as_str() {
        "true" => true,
        "false" => false,
        value => panic!("COLLECTION_STARTの{name}={value}がboolでない"),
    }
}

fn collection_start(path: &Path) -> Stage3cCollectionStart {
    const PREFIX: &str = "CORPUS_3C_COLLECTION_START ";
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}を読めない: {error}", path.display()));
    let payloads: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix(PREFIX))
        .collect();
    assert_eq!(
        payloads.len(),
        1,
        "{}にはCOLLECTION_STARTがexact 1件必要",
        path.display()
    );
    let mut fields = BTreeMap::new();
    for token in payloads[0].split_whitespace() {
        let (name, value) = token
            .split_once('=')
            .unwrap_or_else(|| panic!("COLLECTION_START tokenに=がない: {token}"));
        assert!(
            fields.insert(name.to_owned(), value.to_owned()).is_none(),
            "COLLECTION_START fieldが重複: {name}"
        );
    }
    let start = Stage3cCollectionStart {
        phase: take_collection_start_field(&mut fields, "phase"),
        cases: parse_collection_start_usize(&mut fields, "cases"),
        determinism_repetitions: parse_collection_start_usize(
            &mut fields,
            "determinism_repetitions",
        ),
        performance_repetitions: parse_collection_start_usize(
            &mut fields,
            "performance_repetitions",
        ),
        profile: take_collection_start_field(&mut fields, "profile"),
        functional_search: take_collection_start_field(&mut fields, "functional_search"),
        performance_search: take_collection_start_field(&mut fields, "performance_search"),
        product_watchdog_millis: parse_collection_start_u64(
            &mut fields,
            "product_watchdog_millis",
        ),
        require_recorded_current: parse_collection_start_bool(
            &mut fields,
            "require_recorded_current",
        ),
        target_met_is_not_implied: parse_collection_start_bool(
            &mut fields,
            "target_met_is_not_implied",
        ),
    };
    assert!(
        fields.is_empty(),
        "COLLECTION_STARTに未知fieldがある: {:?}",
        fields.keys().collect::<Vec<_>>()
    );
    start
}

fn assert_collection_start(
    start: &Stage3cCollectionStart,
    expected_phase: &str,
    expected_require_recorded_current: bool,
    manifest: &CorpusManifest,
) {
    assert_eq!(start.phase, expected_phase);
    assert_eq!(start.cases, manifest.planned_slots.len());
    assert_eq!(start.cases, 30);
    assert_eq!(
        start.determinism_repetitions,
        manifest.repetitions.determinism
    );
    assert_eq!(
        start.performance_repetitions,
        manifest.repetitions.performance_release
    );
    assert_eq!(start.profile, "release");
    assert_eq!(start.functional_search, FUNCTIONAL_SEARCH_CONTRACT);
    assert_eq!(start.functional_search, manifest.runner_contract.functional_search());
    assert_eq!(start.performance_search, PERFORMANCE_SEARCH_CONTRACT);
    assert_eq!(
        start.performance_search,
        manifest.runner_contract.performance_search()
    );
    assert_eq!(start.product_watchdog_millis, 30_000);
    assert_eq!(
        start.product_watchdog_millis,
        manifest.runner_contract.product_search_watchdog_millis
    );
    assert_eq!(
        start.require_recorded_current,
        expected_require_recorded_current
    );
    assert!(start.target_met_is_not_implied);
}

fn evidence_file_time_bounds(path: &Path) -> (u64, u64) {
    let metadata = fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}のmetadataを読めない: {error}", path.display()));
    let modified = metadata.modified().expect("evidence log更新時刻");
    let created = metadata.created().unwrap_or(modified);
    (system_time_millis(created), system_time_millis(modified))
}

fn functional_environment_or_metadata(path: &Path) -> (u64, u64) {
    let environment: Option<Stage3cCollectionEnvironment> =
        optional_one_prefixed_json(path, "CORPUS_3C_ENVIRONMENT_JSON=");
    let finished: Option<Stage3cCollectionFinished> =
        optional_one_prefixed_json(path, "CORPUS_3C_FINISHED_JSON=");
    let (started_unix_millis, finished_unix_millis) = match (environment, finished) {
        (Some(environment), Some(finished)) => {
            stage_3c_assert_machine(&environment.machine);
            (
                environment.started_unix_millis,
                finished.finished_unix_millis,
            )
        }
        (None, None) => {
            // 今回のpre-regen 10周はENV/FINISHED追加前のprecompiled binaryで開始した
            // 可能性がある。その1回だけはlog file metadataを完走時刻の補助証跡にする。
            // machineはexact ENVを持つ後続performance logだけから採用する。全証拠の
            // 収集期間にはこのcreated/modifiedも含める。片方だけ存在する不完全logは拒否する。
            evidence_file_time_bounds(path)
        }
        _ => panic!("functional logのENV/FINISHEDが片方しかない"),
    };
    assert!(started_unix_millis > 0);
    assert!(finished_unix_millis >= started_unix_millis);
    (started_unix_millis, finished_unix_millis)
}

fn predeclared_target_contract(manifest: &CorpusManifest) -> Value {
    let cases: Vec<_> = manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
        .map(|case| {
            serde_json::json!({
                "id": &case.id,
                "planned_slot": &case.planned_slot,
                "counts_toward_target": case.counts_toward_target,
                "strata": &case.strata,
                "classification_basis": &case.classification_basis,
                "target": &case.target,
            })
        })
        .collect();
    serde_json::json!({
        "target_case_count": manifest.target_case_count,
        "anchor_case_count": manifest.anchor_case_count,
        "neutral_case_count": manifest.neutral_case_count,
        "pilot_cases_count_toward_target": manifest.pilot_cases_count_toward_target,
        "numeric_policy": &manifest.numeric_policy,
        "gap_weights": &manifest.runner_contract.gap_weights,
        "completion_tolerance": &manifest.runner_contract.completion_tolerance,
        "case_aggregation": &manifest.case_aggregation,
        "classification_contract": &manifest.classification_contract,
        "stratification_plan": &manifest.stratification_plan,
        "planned_slots": &manifest.planned_slots,
        "cases": cases,
    })
}

fn assert_lower_hex_digest(label: &str, value: &str) {
    assert_eq!(value.len(), 16, "{label}: digest長が16でない: {value}");
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "{label}: lowercase hexでない: {value}"
    );
}

fn assert_gap_metric_finite(label: &str, gaps: GapMetric) {
    for (name, value) in [
        ("count", gaps.count),
        ("length", gaps.length),
        ("width", gaps.width),
        ("position", gaps.position),
    ] {
        assert!(value.is_finite(), "{label}.{name}: 非有限値");
    }
}

fn assert_candidate_recorded_current(
    case: &CorpusCase,
    policy: &NumericPolicy,
    functional_fingerprint: &Stage3cFingerprint,
    metrics: &[Option<CandidateMetric>],
) {
    let current = &case.recorded_current;
    assert_eq!(current.outcome, RecordedOutcomeKind::Candidates);
    assert!(current.execution_failure.is_none());
    assert!(current.candidate_count > 0, "{}: 候補が0件", case.id);
    assert_eq!(
        current.candidate_statuses.len(),
        current.candidate_count,
        "{}: candidate status数",
        case.id
    );
    assert_eq!(
        current.stop_reasons.len(),
        current.candidate_count,
        "{}: stop reason数",
        case.id
    );
    assert_eq!(metrics.len(), current.candidate_count, "{}: metric数", case.id);
    assert!(
        current.selected_candidate_index < current.candidate_count,
        "{}: selected indexが候補範囲外",
        case.id
    );
    assert!(
        current
            .candidate_statuses
            .iter()
            .all(|status| matches!(status.as_str(), "checked_to_finish" | "partial" | "no_plan")),
        "{}: 未知のcandidate status",
        case.id
    );
    assert!(
        matches!(
            current.candidate_statuses[current.selected_candidate_index].as_str(),
            "checked_to_finish" | "partial"
        ),
        "{}: selected candidateに利用可能planがない",
        case.id
    );
    assert!(
        current.all_returned_plans_safe,
        "{}: 全返却planの安全性がfalse",
        case.id
    );
    for (index, metric) in metrics.iter().enumerate() {
        let metric = metric
            .as_ref()
            .unwrap_or_else(|| panic!("{}: 候補{index}に安全metricがない", case.id));
        assert_eq!(metric.status, current.candidate_statuses[index]);
        assert_eq!(
            Some(metric.stop_reason.as_str()),
            current.stop_reasons[index].as_deref()
        );
        assert_safety_policy(&metric.safety, policy);
    }
    let recalculated_selected = selected_candidate(metrics, policy)
        .unwrap_or_else(|| panic!("{}: pure選択で安全なplanがない", case.id));
    assert_eq!(
        recalculated_selected, current.selected_candidate_index,
        "{}: selected candidateのpure再計算",
        case.id
    );
    assert_metric_baseline(
        metrics[recalculated_selected]
            .as_ref()
            .expect("selected metric"),
        current,
        policy,
    );
    assert_safety_policy(&current.safety, policy);
    assert_gap_metric_finite("recorded_current.initial_gaps", current.initial_gaps);
    assert_gap_metric_finite("recorded_current.final_gaps", current.final_gaps);
    for (name, value) in [
        ("initial_weighted_gap", current.initial_weighted_gap),
        ("final_weighted_gap", current.final_weighted_gap),
        ("improvement_absolute", current.improvement_absolute),
        ("improvement_ratio", current.improvement_ratio),
    ] {
        assert!(value.is_finite(), "{}: {name}が非有限値", case.id);
    }
    let calculated_improvement = current.initial_weighted_gap - current.final_weighted_gap;
    assert_near(
        "recorded_current.improvement_absolute",
        current.improvement_absolute,
        calculated_improvement,
        policy.weighted_gap_abs_tolerance,
    );
    let calculated_ratio = if current.initial_weighted_gap > 0.0 {
        calculated_improvement / current.initial_weighted_gap
    } else {
        0.0
    };
    // 改善率は「差 / 初期値」の計算小数なので厳密比較しない。既存baseline照合と
    // 同じくweighted gap許容を分子・分母へ1回ずつ伝播したcase固有の幅を使う。
    let ratio_denominator = current
        .initial_weighted_gap
        .abs()
        .max(policy.weighted_gap_abs_tolerance);
    let ratio_tolerance = policy.weighted_gap_abs_tolerance / ratio_denominator
        + calculated_improvement.abs() * policy.weighted_gap_abs_tolerance
            / ratio_denominator.powi(2);
    assert_near(
        "recorded_current.improvement_ratio",
        current.improvement_ratio,
        calculated_ratio,
        ratio_tolerance,
    );
    for (name, value) in [
        (
            "normalized_candidate_hash",
            current.normalized_candidate_hash.as_str(),
        ),
        ("stop_reason_hash", current.stop_reason_hash.as_str()),
        (
            "normalized_result_hash",
            current.normalized_result_hash.as_str(),
        ),
    ] {
        assert_lower_hex_digest(name, value);
    }
    match functional_fingerprint {
        Stage3cFingerprint::Candidates {
            normalized_candidate_hash,
            stop_reason_hash,
            normalized_result_hash,
        } => {
            assert_eq!(
                current.normalized_candidate_hash.as_str(),
                normalized_candidate_hash.as_str()
            );
            assert_eq!(
                current.stop_reason_hash.as_str(),
                stop_reason_hash.as_str()
            );
            assert_eq!(
                current.normalized_result_hash.as_str(),
                normalized_result_hash.as_str()
            );
        }
        Stage3cFingerprint::ExecutionFailure { .. } => {
            panic!("{}: 機能10周のfingerprintが実行失敗", case.id)
        }
    }
    assert_eq!(
        current.time_budget.product_search_watchdog_millis,
        30_000
    );
    assert_recorded_assessment(&case.target, current, policy);
}

fn corpus_status_counts(manifest: &CorpusManifest) -> (usize, usize) {
    let target_cases: Vec<_> = manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
        .collect();
    let failures = target_cases
        .iter()
        .filter(|case| case.recorded_current.outcome == RecordedOutcomeKind::ExecutionFailure)
        .count();
    let target_met = target_cases
        .iter()
        .filter(|case| case.recorded_current.assessment.target_met)
        .count();
    (failures, target_met)
}

fn assert_regenerated_manifest(
    manifest: &CorpusManifest,
    functional_records: &[Stage3cRunRecord],
    baseline_metrics: &[Vec<Option<CandidateMetric>>],
) -> (usize, usize) {
    assert_eq!(manifest.cases.len(), 31);
    assert_eq!(manifest.planned_slots.len(), 30);
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.counts_toward_target)
            .count(),
        30
    );
    let planned_case_ids: BTreeSet<_> = manifest
        .planned_slots
        .iter()
        .map(|slot| slot.case_id.as_str())
        .collect();
    let target_case_ids: BTreeSet<_> = manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
        .map(|case| case.id.as_str())
        .collect();
    assert_eq!(planned_case_ids.len(), 30);
    assert_eq!(target_case_ids.len(), 30);
    assert_eq!(planned_case_ids, target_case_ids);
    assert_eq!(functional_records.len(), 300);
    assert_eq!(baseline_metrics.len(), 30);
    assert_eq!(
        manifest.hash_contract.input_normalization,
        "typed-input-plus-functional-runner-contract-canonical-json-v2"
    );
    assert_eq!(
        manifest.runner_contract.functional_search.as_deref(),
        Some(FUNCTIONAL_SEARCH_CONTRACT)
    );
    assert_eq!(
        manifest.runner_contract.performance_search.as_deref(),
        Some(PERFORMANCE_SEARCH_CONTRACT)
    );
    assert_eq!(
        manifest.runner_contract.product_search_watchdog_millis,
        30_000
    );
    // input契約はtarget判定より先に、非算入pilotを含むmanifest全31件で検証する。
    for case in &manifest.cases {
        let (input_bytes, input) = load_input(case).expect("input fixtureを読めない");
        assert_eq!(
            fixture_checksum(&input_bytes).expect("fixture checksum"),
            case.input.fixture_checksum.digest,
            "{}: fixture checksum",
            case.id
        );
        assert_eq!(
            structure_hash(&input.skeleton, manifest.hash_contract.float_quantum)
                .expect("structure hash"),
            case.input.structure_hash,
            "{}: structure hash",
            case.id
        );
        assert_eq!(
            normalized_hash(
                &NormalizedInputContract::new(&input, &manifest.runner_contract),
                manifest.hash_contract.float_quantum,
            )
            .expect("normalized input hash"),
            case.input.normalized_input_hash,
            "{}: normalized input hash",
            case.id
        );
    }
    let mut must_met = 0;
    let mut partial_met = 0;
    for (case_index, slot) in manifest.planned_slots.iter().enumerate() {
        let case = manifest
            .cases
            .iter()
            .find(|case| case.id == slot.case_id)
            .unwrap_or_else(|| panic!("manifestにcaseがない: {}", slot.case_id));
        assert!(case.counts_toward_target);
        let first_record = &functional_records[case_index];
        assert_eq!(first_record.repetition, 1);
        assert_eq!(first_record.case_id, case.id);
        assert_candidate_recorded_current(
            case,
            &manifest.numeric_policy,
            &first_record.fingerprint,
            &baseline_metrics[case_index],
        );
        if case.recorded_current.assessment.target_met {
            match case.target.class.as_str() {
                "must_complete" => must_met += 1,
                "safe_partial_allowed" => partial_met += 1,
                other => panic!("未知のtarget class: {other}"),
            }
        }
    }
    let (failure_count, target_met) = corpus_status_counts(manifest);
    assert_eq!(failure_count, 0);
    assert_eq!(target_met, 7);
    assert_eq!(must_met, 2);
    assert_eq!(partial_met, 5);
    (failure_count, target_met)
}

fn metrics_fingerprint(fingerprint: &Stage3cFingerprint) -> Stage3cMetricsFingerprint {
    match fingerprint {
        Stage3cFingerprint::Candidates {
            normalized_candidate_hash,
            stop_reason_hash,
            normalized_result_hash,
        } => Stage3cMetricsFingerprint::Candidates {
            normalized_candidate_hash: normalized_candidate_hash.clone(),
            stop_reason_hash: stop_reason_hash.clone(),
            normalized_result_hash: normalized_result_hash.clone(),
        },
        Stage3cFingerprint::ExecutionFailure {
            phase,
            reason,
            normalized_failure_hash,
        } => Stage3cMetricsFingerprint::ExecutionFailure {
            phase: phase.clone(),
            reason: *reason,
            normalized_failure_hash: normalized_failure_hash.clone(),
        },
    }
}

fn performance_outcome(fingerprint: &Stage3cFingerprint) -> Stage3cMetricsPerformanceOutcome {
    match fingerprint {
        Stage3cFingerprint::Candidates { .. } => Stage3cMetricsPerformanceOutcome::Candidates,
        Stage3cFingerprint::ExecutionFailure { phase, reason, .. } => {
            Stage3cMetricsPerformanceOutcome::ExecutionFailure {
                phase: phase.clone(),
                reason: *reason,
            }
        }
    }
}

fn validate_collected_records(
    records: &[Stage3cRunRecord],
    expected_phase: &str,
    repetitions: usize,
    manifest: &CorpusManifest,
    require_stable_fingerprint: bool,
) {
    assert_eq!(records.len(), repetitions * manifest.planned_slots.len());
    for repetition in 1..=repetitions {
        for (case_index, slot) in manifest.planned_slots.iter().enumerate() {
            let record = &records[(repetition - 1) * manifest.planned_slots.len() + case_index];
            let case = manifest
                .cases
                .iter()
                .find(|case| case.id == slot.case_id)
                .unwrap_or_else(|| panic!("manifestにcaseがない: {}", slot.case_id));
            assert_eq!(record.phase, expected_phase);
            assert_eq!(record.repetition, repetition);
            assert_eq!(record.case_index, case_index + 1);
            assert_eq!(record.case_id, slot.case_id);
            assert_eq!(record.planned_slot, slot.slot);
            assert_eq!(record.band, case.strata.band);
            assert_eq!(record.leaf_count, case.strata.leaf_count);
            if require_stable_fingerprint {
                assert!(
                    record.matches_first_in_phase,
                    "{} repetition {repetition}: 機能fingerprint不一致",
                    record.case_id
                );
                assert_eq!(
                    record.fingerprint, records[case_index].fingerprint,
                    "{} repetition {repetition}: 機能fingerprint不一致",
                    record.case_id
                );
            }
        }
    }
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn stage_3c_machine_evidence() -> Stage3cMetricsMachine {
    let rust_verbose = command_text("rustc", &["-vV"]);
    let rust_host = rust_verbose.as_deref().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
    });
    Stage3cMetricsMachine {
        operating_system: Some(std::env::consts::OS.to_owned()),
        architecture: Some(std::env::consts::ARCH.to_owned()),
        logical_cpu_count: thread::available_parallelism().ok().map(usize::from),
        physical_core_count: None,
        cpu_model: None,
        physical_memory_bytes: None,
        rust_version: command_text("rustc", &["-V"]),
        rust_host,
        cargo_version: command_text("cargo", &["-V"]),
        target_dir: std::env::var("CARGO_TARGET_DIR").ok(),
        profile: "release".to_owned(),
    }
}

fn system_time_millis(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(1)
}

fn stage_3c_performance_evidence(
    records: &[Stage3cRunRecord],
    rounds: &[Stage3cRoundRecord],
    manifest: &CorpusManifest,
) -> (
    Vec<Stage3cMetricsCase>,
    Stage3cMetricsPerformance,
    Stage3cMetricsOutliers,
    Stage3cMetricsGateProposal,
) {
    let repetitions = manifest.repetitions.performance_release;
    let mut cases = Vec::new();
    let mut all_values = Vec::new();
    let mut band_values: BTreeMap<String, Vec<Stage3cMetricsElapsed>> = BTreeMap::new();
    let mut outliers = Vec::new();
    let mut case_p95_values = Vec::new();
    for (case_index, slot) in manifest.planned_slots.iter().enumerate() {
        let case = manifest
            .cases
            .iter()
            .find(|case| case.id == slot.case_id)
            .unwrap_or_else(|| panic!("manifestにcaseがない: {}", slot.case_id));
        let mut runs = Vec::new();
        let mut candidate_outcomes = 0;
        let mut watchdog_expired_outcomes = 0;
        let mut elapsed = Vec::new();
        for repetition in 1..=repetitions {
            let record = &records[(repetition - 1) * manifest.planned_slots.len() + case_index];
            let observed_outcome = performance_outcome(&record.fingerprint);
            match &observed_outcome {
                Stage3cMetricsPerformanceOutcome::Candidates => candidate_outcomes += 1,
                Stage3cMetricsPerformanceOutcome::ExecutionFailure { phase, reason } => {
                    assert_eq!(phase, "search");
                    assert_eq!(*reason, AbortKind::WatchdogExpired);
                    watchdog_expired_outcomes += 1;
                }
            }
            elapsed.push(record.elapsed_millis);
            runs.push(Stage3cMetricsPerformanceRun {
                repetition,
                elapsed_millis: record.elapsed_millis,
                observed_outcome,
            });
            let value = Stage3cMetricsElapsed {
                case_id: case.id.clone(),
                repetition,
                elapsed_millis: record.elapsed_millis,
            };
            all_values.push(value.clone());
            band_values
                .entry(case.strata.band.clone())
                .or_default()
                .push(value);
        }
        let q1 = stage_3c_nearest_rank(&elapsed, 25, 100) as f64;
        let q3 = stage_3c_nearest_rank(&elapsed, 75, 100) as f64;
        let iqr = q3 - q1;
        let lower_fence_millis = q1 - 1.5 * iqr;
        let upper_fence_millis = q3 + 1.5 * iqr;
        for run in &runs {
            let value = run.elapsed_millis as f64;
            if value < lower_fence_millis || value > upper_fence_millis {
                outliers.push(Stage3cMetricsOutlier {
                    case_id: case.id.clone(),
                    repetition: run.repetition,
                    elapsed_millis: run.elapsed_millis,
                    lower_fence_millis,
                    upper_fence_millis,
                });
            }
        }
        case_p95_values.push(stage_3c_nearest_rank(&elapsed, 95, 100));
        cases.push(Stage3cMetricsCase {
            case_id: case.id.clone(),
            planned_slot: slot.slot.clone(),
            band: case.strata.band.clone(),
            leaf_count: case.strata.leaf_count,
            recorded_current_fingerprint: stage_3c_metrics_recorded_fingerprint(
                &case.recorded_current,
            ),
            determinism: Stage3cMetricsPhase {
                runs: Vec::new(),
                mismatches_from_first: 0,
                mismatches_from_recorded_current: 0,
            },
            performance: Stage3cMetricsPerformancePhase {
                runs,
                candidate_outcomes,
                watchdog_expired_outcomes,
            },
        });
    }
    let slot_order: BTreeMap<_, _> = manifest
        .planned_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (slot.case_id.as_str(), index))
        .collect();
    all_values.sort_by_key(|value| {
        (
            value.repetition,
            *slot_order
                .get(value.case_id.as_str())
                .expect("performance case is a planned slot"),
        )
    });
    let all_elapsed: Vec<_> = all_values
        .iter()
        .map(|value| value.elapsed_millis)
        .collect();
    let mut bands = Vec::new();
    for band in ["A", "B", "C", "D", "E"] {
        let mut values = band_values.remove(band).expect("5帯の性能値");
        values.sort_by_key(|value| {
            (
                value.repetition,
                *slot_order
                    .get(value.case_id.as_str())
                    .expect("performance case is a planned slot"),
            )
        });
        let elapsed: Vec<_> = values.iter().map(|value| value.elapsed_millis).collect();
        bands.push(Stage3cMetricsBand {
            band: band.to_owned(),
            median_millis: stage_3c_median_millis(&elapsed),
            p95_millis: stage_3c_nearest_rank(&elapsed, 95, 100),
            values,
        });
    }
    assert!(band_values.is_empty());
    let corpus_rounds: Vec<_> = rounds
        .iter()
        .map(|round| Stage3cMetricsCorpusRound {
            repetition: round.repetition,
            product_elapsed_millis: round.phase_elapsed_millis,
            collector_wall_elapsed_millis: round.collector_wall_elapsed_millis,
        })
        .collect();
    let corpus_elapsed: Vec<_> = corpus_rounds
        .iter()
        .map(|round| round.phase_elapsed_millis)
        .collect();
    let performance = Stage3cMetricsPerformance {
        all_median_millis: stage_3c_median_millis(&all_elapsed),
        all_p95_millis: stage_3c_nearest_rank(&all_elapsed, 95, 100),
        bands,
        corpus_median_millis: stage_3c_median_millis(&corpus_elapsed),
        corpus_p95_millis: stage_3c_nearest_rank(&corpus_elapsed, 95, 100),
        corpus_rounds,
        all_values,
    };
    let outliers = Stage3cMetricsOutliers {
        method: "tukey_per_case_nearest_rank_1_5_iqr".to_owned(),
        scope: "each_case_five_release_values".to_owned(),
        excluded_from_aggregates: false,
        values: outliers,
    };
    let reference_p95 = performance.corpus_p95_millis;
    let (raw_gate_millis, proposed_gate_seconds, proposed_gate_millis) =
        stage_3c_gate_from_p95(reference_p95);
    let case_reference_p95 = case_p95_values
        .into_iter()
        .max()
        .expect("30 case P95 values");
    let (case_raw_gate_millis, proposed_case_gate_seconds, proposed_case_gate_millis) =
        stage_3c_gate_from_p95(case_reference_p95);
    let gate = Stage3cMetricsGateProposal {
        source: "sum_of_case_elapsed_millis_p95".to_owned(),
        performance_baseline_fraction: manifest
            .numeric_policy
            .performance_baseline_fraction_of_gate,
        reference_p95_millis: reference_p95,
        raw_gate_millis,
        proposed_gate_seconds,
        proposed_gate_millis,
        case_source: "maximum_case_p95_millis".to_owned(),
        case_reference_p95_millis: case_reference_p95,
        case_raw_gate_millis,
        proposed_case_gate_seconds,
        proposed_case_gate_millis,
        enforced: false,
        status: "awaiting_coordinator".to_owned(),
    };
    (cases, performance, outliers, gate)
}

fn remove_staged_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}を消せない: {error}", path.display())),
    }
}

fn preflight_canonical_pair(
    manifest_file: &Path,
    metrics_file: &Path,
    manifest_bytes: &[u8],
    metrics_bytes: &[u8],
) {
    // 正本へ1 byteも書く前に、候補pairと現在の30 inputだけから通常validatorと
    // 同じ完全検証を行う。staged readbackは、この純粋検証済みbytesが実際に同じ
    // bytesとして書けることだけを続けて確かめる。
    let candidate_manifest: CorpusManifest =
        serde_json::from_slice(manifest_bytes).expect("candidate manifest schema");
    let candidate_metrics: Stage3cMetricsFixture =
        serde_json::from_slice(metrics_bytes).expect("candidate metrics schema");
    let mut candidate_immutable = BTreeMap::new();
    assert!(
        candidate_immutable
            .insert(manifest_file.to_path_buf(), manifest_bytes.to_vec())
            .is_none()
    );
    assert!(
        candidate_immutable
            .insert(metrics_file.to_path_buf(), metrics_bytes.to_vec())
            .is_none()
    );
    for slot in &candidate_manifest.planned_slots {
        let case = candidate_manifest
            .cases
            .iter()
            .find(|case| case.id == slot.case_id)
            .unwrap_or_else(|| panic!("{}: candidate target caseがない", slot.case_id));
        let path = fixture_path(&case.input.fixture).expect("candidate input path");
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("{}を読めない: {error}", path.display()));
        assert!(
            candidate_immutable.insert(path, bytes).is_none(),
            "candidate input pathが重複"
        );
    }
    assert_eq!(candidate_immutable.len(), 32);
    stage_3c_assert_metrics_fixture(
        &candidate_metrics,
        &candidate_manifest,
        &candidate_immutable,
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_nanos();
    let manifest_name = manifest_file
        .file_name()
        .and_then(|name| name.to_str())
        .expect("manifest file name");
    let metrics_name = metrics_file
        .file_name()
        .and_then(|name| name.to_str())
        .expect("metrics file name");
    let manifest_staged = manifest_file.with_file_name(format!(
        ".{manifest_name}.{}.{}.staged",
        std::process::id(),
        nonce
    ));
    let metrics_staged = metrics_file.with_file_name(format!(
        ".{metrics_name}.{}.{}.staged",
        std::process::id(),
        nonce
    ));
    let staged = (|| -> Result<(), String> {
        fs::write(&manifest_staged, manifest_bytes)
            .map_err(|error| format!("{}を書けない: {error}", manifest_staged.display()))?;
        fs::write(&metrics_staged, metrics_bytes)
            .map_err(|error| format!("{}を書けない: {error}", metrics_staged.display()))?;
        let manifest_readback = fs::read(&manifest_staged)
            .map_err(|error| format!("{}を再読込できない: {error}", manifest_staged.display()))?;
        let metrics_readback = fs::read(&metrics_staged)
            .map_err(|error| format!("{}を再読込できない: {error}", metrics_staged.display()))?;
        if manifest_readback.as_slice() != manifest_bytes {
            return Err(format!("{}のreadbackが不一致", manifest_staged.display()));
        }
        if metrics_readback.as_slice() != metrics_bytes {
            return Err(format!("{}のreadbackが不一致", metrics_staged.display()));
        }
        let _: CorpusManifest = serde_json::from_slice(&manifest_readback)
            .map_err(|error| format!("staged manifest schema: {error}"))?;
        let _: Stage3cMetricsFixture = serde_json::from_slice(&metrics_readback)
            .map_err(|error| format!("staged metrics schema: {error}"))?;
        Ok(())
    })();
    let cleanup_manifest = remove_staged_file(&manifest_staged);
    let cleanup_metrics = remove_staged_file(&metrics_staged);
    if let Err(error) = staged {
        panic!(
            "正本pairの同directory preflight失敗: {error}; cleanup_manifest={cleanup_manifest:?}; cleanup_metrics={cleanup_metrics:?}"
        );
    }
    cleanup_manifest.unwrap_or_else(|error| panic!("staged manifest cleanup失敗: {error}"));
    cleanup_metrics.unwrap_or_else(|error| panic!("staged metrics cleanup失敗: {error}"));
}

fn write_canonical_pair(
    manifest_bytes: &[u8],
    metrics_bytes: &[u8],
    original_manifest: &[u8],
    original_metrics: &[u8],
) {
    let manifest_file = manifest_path();
    let metrics_file = stage_3c_metrics_path();
    preflight_canonical_pair(
        &manifest_file,
        &metrics_file,
        manifest_bytes,
        metrics_bytes,
    );
    // CAS: evidenceを読んだ後に別担当が正本を変えた場合、その変更へ上書きしない。
    // 2ファイルとも、write直前に最初のsnapshotとbyte完全一致することを要求する。
    assert_eq!(
        fs::read(&manifest_file)
            .expect("write直前manifestを読めない")
            .as_slice(),
        original_manifest,
        "write直前にmanifestが変わった"
    );
    assert_eq!(
        fs::read(&metrics_file)
            .expect("write直前metricsを読めない")
            .as_slice(),
        original_metrics,
        "write直前にmetricsが変わった"
    );
    let replacement = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fs::write(&manifest_file, manifest_bytes)
            .unwrap_or_else(|error| panic!("{}を書けない: {error}", manifest_file.display()));
        fs::write(&metrics_file, metrics_bytes)
            .unwrap_or_else(|error| panic!("{}を書けない: {error}", metrics_file.display()));
        assert_eq!(
            fs::read(&manifest_file)
                .expect("更新manifestを再読込できない")
                .as_slice(),
            manifest_bytes,
            "更新manifestのreadbackが不一致"
        );
        assert_eq!(
            fs::read(&metrics_file)
                .expect("更新metricsを再読込できない")
                .as_slice(),
            metrics_bytes,
            "更新metricsのreadbackが不一致"
        );
        let (_, manifest) = load_manifest().expect("再生成manifestを読めない");
        let immutable = stage_3c_immutable_files(&manifest);
        let metrics: Stage3cMetricsFixture = serde_json::from_slice(
            immutable
                .get(&metrics_file)
                .expect("再生成metricsをsnapshotできない"),
        )
        .expect("再生成metrics schema");
        stage_3c_assert_metrics_fixture(&metrics, &manifest, &immutable);
    }));
    if let Err(payload) = replacement {
        let manifest_rollback = fs::write(&manifest_file, original_manifest);
        let metrics_rollback = fs::write(&metrics_file, original_metrics);
        if manifest_rollback.is_err() || metrics_rollback.is_err() {
            panic!(
                "正本pair rollback失敗: manifest={manifest_rollback:?}; metrics={metrics_rollback:?}"
            );
        }
        assert_eq!(
            fs::read(&manifest_file)
                .expect("rollback manifestを読めない")
                .as_slice(),
            original_manifest,
            "rollback manifest byte不一致"
        );
        assert_eq!(
            fs::read(&metrics_file)
                .expect("rollback metricsを読めない")
                .as_slice(),
            original_metrics,
            "rollback metrics byte不一致"
        );
        std::panic::resume_unwind(payload);
    }
}

fn regenerate_corpus_from_evidence() {
    let functional_log = required_evidence_path("ORI3_CORPUS_FUNCTIONAL_LOG");
    let baseline_log = required_evidence_path("ORI3_CORPUS_BASELINE_LOG");
    let performance_log = required_evidence_path("ORI3_CORPUS_PERFORMANCE_LOG");
    let manifest_file = manifest_path();
    let metrics_file = stage_3c_metrics_path();
    let original_manifest = fs::read(&manifest_file).expect("旧manifestを読めない");
    let original_metrics = fs::read(&metrics_file).expect("旧metricsを読めない");
    let (_, mut manifest) = load_manifest().expect("旧manifestを解釈できない");
    let target_contract_before = predeclared_target_contract(&manifest);
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.counts_toward_target)
            .count(),
        30
    );
    let (old_failure_count, old_target_met) = corpus_status_counts(&manifest);
    assert_eq!(old_failure_count, 9);
    assert_eq!(old_target_met, 6);
    let functional_start = collection_start(&functional_log);
    assert_collection_start(
        &functional_start,
        "functional_determinism",
        false,
        &manifest,
    );
    let (functional_started_unix_millis, functional_finished_unix_millis) =
        functional_environment_or_metadata(&functional_log);
    let (baseline_started_unix_millis, baseline_finished_unix_millis) =
        evidence_file_time_bounds(&baseline_log);
    let performance_start = collection_start(&performance_log);
    assert_collection_start(
        &performance_start,
        "product_performance",
        false,
        &manifest,
    );
    let performance_environment: Stage3cCollectionEnvironment = exactly_one_prefixed_json(
        &performance_log,
        "CORPUS_3C_ENVIRONMENT_JSON=",
    );
    let performance_finished: Stage3cCollectionFinished = exactly_one_prefixed_json(
        &performance_log,
        "CORPUS_3C_FINISHED_JSON=",
    );
    assert!(performance_environment.started_unix_millis > 0);
    assert!(
        performance_finished.finished_unix_millis
            >= performance_environment.started_unix_millis
    );
    stage_3c_assert_machine(&performance_environment.machine);
    let input_snapshots: BTreeMap<_, _> = manifest
        .cases
        .iter()
        .map(|case| {
            let path = fixture_path(&case.input.fixture).expect("input fixture path");
            let bytes = fs::read(&path)
                .unwrap_or_else(|error| panic!("{}を読めない: {error}", path.display()));
            (path, bytes)
        })
        .collect();

    let functional_records: Vec<Stage3cRunRecord> =
        prefixed_json_records(&functional_log, "CORPUS_3C_RUN_JSON=");
    validate_collected_records(
        &functional_records,
        "functional_determinism",
        manifest.repetitions.determinism,
        &manifest,
        true,
    );
    let functional_summary: Stage3cCollectionSummary =
        exactly_one_prefixed_json(&functional_log, "CORPUS_3C_SUMMARY_JSON=");
    assert_eq!(functional_summary.determinism_run_count, 300);
    assert_eq!(functional_summary.determinism_mismatches_from_first, 0);
    assert_eq!(functional_summary.performance_run_count, 0);
    assert_eq!(functional_summary.performance_mismatches_from_first, 0);
    assert_eq!(
        functional_summary.performance_mismatches_from_recorded_current,
        0
    );

    let baseline_metrics: Vec<Vec<Option<CandidateMetric>>> =
        prefixed_json_records(&baseline_log, "CORPUS_3B_METRICS=");
    assert_eq!(baseline_metrics.len(), manifest.planned_slots.len());
    let evidence_cases: Vec<CorpusCase> =
        prefixed_json_records(&baseline_log, "CORPUS_3B_CASE_JSON=");
    assert_eq!(evidence_cases.len(), manifest.planned_slots.len());
    for (evidence, slot) in evidence_cases.iter().zip(&manifest.planned_slots) {
        assert_eq!(evidence.id, slot.case_id, "baseline evidenceの順序");
    }
    let mut evidence_by_id: BTreeMap<_, _> = evidence_cases
        .into_iter()
        .map(|case| (case.id.clone(), case))
        .collect();
    assert_eq!(evidence_by_id.len(), manifest.planned_slots.len());
    for (case_index, slot) in manifest.planned_slots.iter().enumerate() {
        let evidence = evidence_by_id
            .remove(&slot.case_id)
            .unwrap_or_else(|| panic!("{}のbaseline evidenceがない", slot.case_id));
        let case = manifest
            .cases
            .iter_mut()
            .find(|case| case.id == slot.case_id)
            .unwrap_or_else(|| panic!("manifestにcaseがない: {}", slot.case_id));
        assert_eq!(evidence.input.fixture, case.input.fixture);
        assert_eq!(
            evidence.input.fixture_checksum.digest,
            case.input.fixture_checksum.digest
        );
        assert_eq!(evidence.input.structure_hash, case.input.structure_hash);
        assert_eq!(evidence.target.class, case.target.class);
        assert_eq!(evidence.target.criterion, case.target.criterion);
        assert_eq!(
            evidence.recorded_current.outcome,
            RecordedOutcomeKind::Candidates
        );
        assert!(evidence.recorded_current.execution_failure.is_none());
        assert!(evidence.recorded_current.candidate_count > 0);
        assert!(evidence.recorded_current.all_returned_plans_safe);
        assert_candidate_recorded_current(
            &evidence,
            &manifest.numeric_policy,
            &functional_records[case_index].fingerprint,
            &baseline_metrics[case_index],
        );
        assert_eq!(
            stage_3c_metrics_recorded_fingerprint(&evidence.recorded_current),
            metrics_fingerprint(&functional_records[case_index].fingerprint)
        );
        case.recorded_current = evidence.recorded_current;
    }
    assert!(evidence_by_id.is_empty());

    manifest.runner_contract.functional_search = Some(FUNCTIONAL_SEARCH_CONTRACT.to_owned());
    manifest.runner_contract.performance_search = Some(PERFORMANCE_SEARCH_CONTRACT.to_owned());
    manifest.hash_contract.input_normalization =
        "typed-input-plus-functional-runner-contract-canonical-json-v2".to_owned();
    assert_eq!(
        manifest.runner_contract.product_search_watchdog_millis,
        30_000
    );
    for case in &mut manifest.cases {
        let (input_bytes, input) = load_input(case).expect("input fixtureを読めない");
        assert_eq!(
            fixture_checksum(&input_bytes).expect("fixture checksum"),
            case.input.fixture_checksum.digest
        );
        assert_eq!(
            structure_hash(&input.skeleton, manifest.hash_contract.float_quantum)
                .expect("structure hash"),
            case.input.structure_hash
        );
        case.input.normalized_input_hash = normalized_hash(
            &NormalizedInputContract::new(&input, &manifest.runner_contract),
            manifest.hash_contract.float_quantum,
        )
        .expect("normalized input hash");
        case.recorded_current
            .time_budget
            .product_search_watchdog_millis = 30_000;
    }

    let performance_records: Vec<Stage3cRunRecord> =
        prefixed_json_records(&performance_log, "CORPUS_3C_RUN_JSON=");
    validate_collected_records(
        &performance_records,
        "product_performance",
        manifest.repetitions.performance_release,
        &manifest,
        false,
    );
    let performance_rounds: Vec<Stage3cRoundRecord> =
        prefixed_json_records(&performance_log, "CORPUS_3C_ROUND_JSON=");
    assert_eq!(
        performance_rounds.len(),
        manifest.repetitions.performance_release
    );
    for (index, round) in performance_rounds.iter().enumerate() {
        assert_eq!(round.phase, "product_performance");
        assert_eq!(round.repetition, index + 1);
        assert_eq!(round.case_count, manifest.planned_slots.len());
    }
    let performance_summary: Stage3cCollectionSummary =
        exactly_one_prefixed_json(&performance_log, "CORPUS_3C_SUMMARY_JSON=");
    assert_eq!(performance_summary.determinism_run_count, 0);
    assert_eq!(performance_summary.determinism_mismatches_from_first, 0);
    assert_eq!(
        performance_summary.determinism_mismatches_from_recorded_current,
        0
    );
    assert_eq!(performance_summary.performance_run_count, 150);

    for (case_index, slot) in manifest.planned_slots.iter().enumerate() {
        let case = manifest
            .cases
            .iter_mut()
            .find(|case| case.id == slot.case_id)
            .expect("target case");
        case.recorded_current
            .time_budget
            .measured_release_elapsed_millis = Some(performance_records[case_index].elapsed_millis);
        case.recorded_current.time_budget.basis = format!(
            "Stage 3-C product-path performance repetition 1 observed {}ms with the exact 30000ms per-search watchdog. Functional recorded_current uses deterministic search without a wall-clock cutoff. The historical 10000ms case and 300000ms corpus limits remain non-enforced pending coordinator gate approval; the full five-run evidence is stored separately.",
            performance_records[case_index].elapsed_millis
        );
    }
    assert_eq!(
        target_contract_before,
        predeclared_target_contract(&manifest)
    );
    for (path, before) in &input_snapshots {
        assert_eq!(
            fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
            *before,
            "再生成準備がinput fixtureを変えた: {}",
            path.display()
        );
    }

    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest JSON");
    manifest_bytes.push(b'\n');
    let parsed_manifest: CorpusManifest =
        serde_json::from_slice(&manifest_bytes).expect("再生成manifest schema");
    assert_eq!(
        target_contract_before,
        predeclared_target_contract(&parsed_manifest)
    );
    let (new_failure_count, new_target_met) = assert_regenerated_manifest(
        &parsed_manifest,
        &functional_records,
        &baseline_metrics,
    );
    let (mut cases, performance, outliers, gate_proposal) =
        stage_3c_performance_evidence(&performance_records, &performance_rounds, &parsed_manifest);
    for (case_index, case_metrics) in cases.iter_mut().enumerate() {
        let recorded = &case_metrics.recorded_current_fingerprint;
        let first = metrics_fingerprint(&functional_records[case_index].fingerprint);
        assert_eq!(&first, recorded);
        case_metrics.determinism.runs = (1..=parsed_manifest.repetitions.determinism)
            .map(|repetition| {
                let record = &functional_records
                    [(repetition - 1) * parsed_manifest.planned_slots.len() + case_index];
                let fingerprint = metrics_fingerprint(&record.fingerprint);
                assert_eq!(&fingerprint, recorded);
                Stage3cMetricsRun {
                    repetition,
                    matches_first: true,
                    matches_recorded_current: true,
                    fingerprint,
                }
            })
            .collect();
    }
    let target_cases: Vec<_> = parsed_manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
        .collect();
    let must_cases: Vec<_> = target_cases
        .iter()
        .filter(|case| case.target.class == "must_complete")
        .collect();
    let partial_cases: Vec<_> = target_cases
        .iter()
        .filter(|case| case.target.class == "safe_partial_allowed")
        .collect();
    let target_summary = Stage3cMetricsTargetSummary {
        basis: "functional_recorded_current_acceptance".to_owned(),
        target_met: target_cases
            .iter()
            .filter(|case| case.recorded_current.assessment.target_met)
            .count(),
        target_unmet: target_cases
            .iter()
            .filter(|case| !case.recorded_current.assessment.target_met)
            .count(),
        must_complete_met: must_cases
            .iter()
            .filter(|case| case.recorded_current.assessment.target_met)
            .count(),
        must_complete_total: must_cases.len(),
        safe_partial_met: partial_cases
            .iter()
            .filter(|case| case.recorded_current.assessment.target_met)
            .count(),
        safe_partial_total: partial_cases.len(),
    };
    let input_integrity = parsed_manifest
        .planned_slots
        .iter()
        .map(|slot| {
            let case = parsed_manifest
                .cases
                .iter()
                .find(|case| case.id == slot.case_id)
                .expect("target case");
            Stage3cMetricsInputChecksum {
                case_id: case.id.clone(),
                checksum: case.input.fixture_checksum.digest.clone(),
            }
        })
        .collect();
    let measurement_started_unix_millis = functional_started_unix_millis
        .min(baseline_started_unix_millis)
        .min(performance_environment.started_unix_millis);
    let measurement_finished_unix_millis = functional_finished_unix_millis
        .max(baseline_finished_unix_millis)
        .max(performance_finished.finished_unix_millis);
    let metrics = Stage3cMetricsFixture {
        schema_version: 2,
        corpus_id: parsed_manifest.corpus_id.clone(),
        stage: "3-C".to_owned(),
        measurement_started_unix_millis,
        measurement_finished_unix_millis,
        runner: Stage3cMetricsRunner {
            profile: "release".to_owned(),
            determinism_repetitions: parsed_manifest.repetitions.determinism,
            performance_repetitions: parsed_manifest.repetitions.performance_release,
            functional_search: FUNCTIONAL_SEARCH_CONTRACT.to_owned(),
            performance_search: PERFORMANCE_SEARCH_CONTRACT.to_owned(),
            product_search_watchdog_millis: 30_000,
        },
        fixture_integrity: Stage3cMetricsFixtureIntegrity {
            algorithm: "fnv1a64".to_owned(),
            manifest_checksum: fixture_checksum(&manifest_bytes).expect("manifest checksum"),
            inputs: input_integrity,
        },
        machine: performance_environment.machine,
        cases,
        performance,
        outliers,
        gate_proposal,
        target_summary,
    };
    let mut metrics_bytes = serde_json::to_vec_pretty(&metrics).expect("metrics JSON");
    metrics_bytes.push(b'\n');
    let _: Stage3cMetricsFixture =
        serde_json::from_slice(&metrics_bytes).expect("再生成metrics schema");
    write_canonical_pair(
        &manifest_bytes,
        &metrics_bytes,
        &original_manifest,
        &original_metrics,
    );
    println!(
        "CORPUS_REGENERATION_COMPLETE cases=30 old_failure_count={} new_failure_count={} old_target_met={} new_target_met={} target_contract_unchanged=true target_inputs_unchanged=30 pilot_inputs_unchanged=1 functional_runs={} performance_runs={}",
        old_failure_count,
        new_failure_count,
        old_target_met,
        new_target_met,
        functional_summary.determinism_run_count,
        performance_summary.performance_run_count,
    );
}

fn stage_3c_assert_machine(machine: &Stage3cMetricsMachine) {
    assert_eq!(machine.profile, "release");
    let strings = [
        machine.operating_system.as_deref(),
        machine.architecture.as_deref(),
        machine.cpu_model.as_deref(),
        machine.rust_version.as_deref(),
        machine.rust_host.as_deref(),
        machine.cargo_version.as_deref(),
        machine.target_dir.as_deref(),
    ];
    for value in strings.into_iter().flatten() {
        assert!(
            !value.trim().is_empty(),
            "machine information must not be blank"
        );
    }
    for value in [machine.logical_cpu_count, machine.physical_core_count]
        .into_iter()
        .flatten()
    {
        assert!(value > 0, "machine CPU count must be positive");
    }
    if let Some(bytes) = machine.physical_memory_bytes {
        assert!(bytes > 0, "machine physical memory must be positive");
    }
    assert!(
        strings
            .into_iter()
            .flatten()
            .any(|value| !value.trim().is_empty())
            || machine.logical_cpu_count.is_some()
            || machine.physical_core_count.is_some()
            || machine.physical_memory_bytes.is_some(),
        "machine information must contain at least one observed value"
    );
}

#[derive(Clone, Copy, Debug)]
struct Stage3cMismatchCounts {
    from_first: usize,
    from_recorded_current: usize,
}

fn stage_3c_assert_phase(
    phase: &Stage3cMetricsPhase,
    repetitions: usize,
    recorded: &Stage3cMetricsFingerprint,
    case_id: &str,
) -> Stage3cMismatchCounts {
    assert_eq!(phase.runs.len(), repetitions, "{case_id}: repetition count");
    let first = phase
        .runs
        .first()
        .map(|run| &run.fingerprint)
        .expect("nonzero repetitions require a first fingerprint");
    let mut mismatches_from_first = 0;
    let mut mismatches_from_recorded = 0;
    for (index, run) in phase.runs.iter().enumerate() {
        assert_eq!(run.repetition, index + 1, "{case_id}: repetition order");
        let matches_first = &run.fingerprint == first;
        let matches_recorded = &run.fingerprint == recorded;
        assert_eq!(
            run.matches_first, matches_first,
            "{case_id}: first mismatch flag"
        );
        assert_eq!(
            run.matches_recorded_current, matches_recorded,
            "{case_id}: recorded-current mismatch flag"
        );
        if !matches_first {
            mismatches_from_first += 1;
        }
        if !matches_recorded {
            mismatches_from_recorded += 1;
        }
    }
    assert_eq!(phase.mismatches_from_first, mismatches_from_first);
    assert_eq!(
        phase.mismatches_from_recorded_current,
        mismatches_from_recorded
    );
    Stage3cMismatchCounts {
        from_first: mismatches_from_first,
        from_recorded_current: mismatches_from_recorded,
    }
}

fn stage_3c_immutable_files(manifest: &CorpusManifest) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for path in [manifest_path(), stage_3c_metrics_path()] {
        let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(files.insert(path, bytes).is_none());
    }
    for slot in &manifest.planned_slots {
        let case = manifest
            .cases
            .iter()
            .find(|case| case.id == slot.case_id)
            .unwrap_or_else(|| panic!("{}: materialized case is missing", slot.case_id));
        let path = fixture_path(&case.input.fixture).expect("target fixture path");
        let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            files.insert(path, bytes).is_none(),
            "duplicate target fixture"
        );
    }
    assert_eq!(files.len(), 32, "manifest, metrics, and 30 target inputs");
    files
}

fn stage_3c_assert_fixture_integrity(
    integrity: &Stage3cMetricsFixtureIntegrity,
    manifest: &CorpusManifest,
    immutable: &BTreeMap<PathBuf, Vec<u8>>,
) {
    assert_eq!(integrity.algorithm, "fnv1a64");
    let manifest_bytes = immutable
        .get(&manifest_path())
        .expect("manifest bytes were snapshotted");
    assert_eq!(
        integrity.manifest_checksum,
        fixture_checksum(manifest_bytes).expect("manifest checksum")
    );
    assert_eq!(integrity.inputs.len(), manifest.planned_slots.len());
    for (entry, slot) in integrity.inputs.iter().zip(&manifest.planned_slots) {
        assert_eq!(entry.case_id, slot.case_id);
        let case = manifest
            .cases
            .iter()
            .find(|case| case.id == slot.case_id)
            .expect("materialized case");
        let path = fixture_path(&case.input.fixture).expect("fixture path");
        let bytes = immutable.get(&path).expect("input bytes were snapshotted");
        let checksum = fixture_checksum(bytes).expect("input checksum");
        assert_eq!(entry.checksum, checksum, "{}: metrics checksum", case.id);
        assert_eq!(
            checksum, case.input.fixture_checksum.digest,
            "{}: manifest checksum",
            case.id
        );
    }
}

fn stage_3c_assert_target_summary(
    summary: &Stage3cMetricsTargetSummary,
    manifest: &CorpusManifest,
) {
    assert_eq!(summary.basis, "functional_recorded_current_acceptance");
    let target_cases: Vec<_> = manifest
        .cases
        .iter()
        .filter(|case| case.counts_toward_target)
        .collect();
    let target_met = target_cases
        .iter()
        .filter(|case| case.recorded_current.assessment.target_met)
        .count();
    let must_cases: Vec<_> = target_cases
        .iter()
        .filter(|case| case.target.class == "must_complete")
        .collect();
    let partial_cases: Vec<_> = target_cases
        .iter()
        .filter(|case| case.target.class == "safe_partial_allowed")
        .collect();
    assert_eq!(summary.target_met, target_met);
    assert_eq!(summary.target_unmet, target_cases.len() - target_met);
    assert_eq!(summary.must_complete_total, must_cases.len());
    assert_eq!(
        summary.must_complete_met,
        must_cases
            .iter()
            .filter(|case| case.recorded_current.assessment.target_met)
            .count()
    );
    assert_eq!(summary.safe_partial_total, partial_cases.len());
    assert_eq!(
        summary.safe_partial_met,
        partial_cases
            .iter()
            .filter(|case| case.recorded_current.assessment.target_met)
            .count()
    );
    assert_eq!((summary.target_met, summary.target_unmet), (7, 23));
    assert_eq!(
        (summary.must_complete_met, summary.must_complete_total),
        (2, 12)
    );
    assert_eq!(
        (summary.safe_partial_met, summary.safe_partial_total),
        (5, 18)
    );
}

fn stage_3c_assert_metrics_fixture(
    metrics: &Stage3cMetricsFixture,
    manifest: &CorpusManifest,
    immutable: &BTreeMap<PathBuf, Vec<u8>>,
) {
    assert_eq!(metrics.schema_version, 2);
    assert_eq!(metrics.corpus_id, manifest.corpus_id);
    assert_eq!(metrics.corpus_id, "proposal-benchmark-corpus-v2");
    assert_eq!(metrics.stage, "3-C");
    assert!(metrics.measurement_started_unix_millis > 0);
    assert!(metrics.measurement_finished_unix_millis >= metrics.measurement_started_unix_millis);
    assert_eq!(metrics.runner.profile, "release");
    assert_eq!(metrics.runner.determinism_repetitions, 10);
    assert_eq!(metrics.runner.performance_repetitions, 5);
    assert_eq!(
        metrics.runner.determinism_repetitions,
        manifest.repetitions.determinism
    );
    assert_eq!(
        metrics.runner.performance_repetitions,
        manifest.repetitions.performance_release
    );
    assert_eq!(
        metrics.runner.functional_search,
        manifest.runner_contract.functional_search()
    );
    assert_eq!(
        metrics.runner.performance_search,
        manifest.runner_contract.performance_search()
    );
    assert_eq!(
        metrics.runner.product_search_watchdog_millis,
        manifest.runner_contract.product_search_watchdog_millis
    );
    assert_eq!(metrics.runner.product_search_watchdog_millis, 30_000);
    stage_3c_assert_fixture_integrity(&metrics.fixture_integrity, manifest, immutable);
    stage_3c_assert_machine(&metrics.machine);
    assert_eq!(metrics.cases.len(), manifest.planned_slots.len());

    let mut expected_all_values = Vec::new();
    let mut expected_band_values: BTreeMap<&str, Vec<Stage3cMetricsElapsed>> = BTreeMap::new();
    let mut case_p95_values = Vec::new();
    let mut determinism_mismatch_cases = Vec::new();
    let mut determinism_recorded_mismatch_cases = Vec::new();
    for (case_metrics, slot) in metrics.cases.iter().zip(&manifest.planned_slots) {
        assert_eq!(case_metrics.case_id, slot.case_id);
        assert_eq!(case_metrics.planned_slot, slot.slot);
        let case = manifest
            .cases
            .iter()
            .find(|case| case.id == slot.case_id)
            .expect("materialized case");
        assert_eq!(case_metrics.band, case.strata.band);
        assert_eq!(case_metrics.leaf_count, case.strata.leaf_count);
        let recorded = stage_3c_metrics_recorded_fingerprint(&case.recorded_current);
        assert_eq!(case_metrics.recorded_current_fingerprint, recorded);
        let determinism_mismatches = stage_3c_assert_phase(
            &case_metrics.determinism,
            metrics.runner.determinism_repetitions,
            &case_metrics.recorded_current_fingerprint,
            &case_metrics.case_id,
        );
        if determinism_mismatches.from_first > 0 {
            determinism_mismatch_cases.push(case_metrics.case_id.as_str());
        }
        if determinism_mismatches.from_recorded_current > 0 {
            determinism_recorded_mismatch_cases.push(case_metrics.case_id.as_str());
        }
        assert_eq!(
            case_metrics.performance.runs.len(),
            metrics.runner.performance_repetitions,
            "{}: performance repetition count",
            case_metrics.case_id
        );
        let mut candidate_outcomes = 0;
        let mut watchdog_expired_outcomes = 0;
        for (index, run) in case_metrics.performance.runs.iter().enumerate() {
            assert_eq!(
                run.repetition,
                index + 1,
                "{}: performance repetition order",
                case_metrics.case_id
            );
            match &run.observed_outcome {
                Stage3cMetricsPerformanceOutcome::Candidates => candidate_outcomes += 1,
                Stage3cMetricsPerformanceOutcome::ExecutionFailure { phase, reason } => {
                    assert_eq!(phase, "search");
                    assert_eq!(*reason, AbortKind::WatchdogExpired);
                    watchdog_expired_outcomes += 1;
                }
            }
        }
        assert_eq!(
            case_metrics.performance.candidate_outcomes,
            candidate_outcomes
        );
        assert_eq!(
            case_metrics.performance.watchdog_expired_outcomes,
            watchdog_expired_outcomes
        );
        assert_eq!(
            candidate_outcomes + watchdog_expired_outcomes,
            metrics.runner.performance_repetitions
        );
        let elapsed: Vec<_> = case_metrics
            .performance
            .runs
            .iter()
            .map(|run| run.elapsed_millis)
            .collect();
        case_p95_values.push(stage_3c_nearest_rank(&elapsed, 95, 100));
    }
    for repetition in 1..=metrics.runner.performance_repetitions {
        for case_metrics in &metrics.cases {
            let run = &case_metrics.performance.runs[repetition - 1];
            let value = Stage3cMetricsElapsed {
                case_id: case_metrics.case_id.clone(),
                repetition,
                elapsed_millis: run.elapsed_millis,
            };
            expected_band_values
                .entry(case_metrics.band.as_str())
                .or_default()
                .push(value.clone());
            expected_all_values.push(value);
        }
    }
    assert_eq!(metrics.performance.all_values, expected_all_values);
    assert_eq!(metrics.performance.all_values.len(), 150);
    let all_elapsed: Vec<_> = metrics
        .performance
        .all_values
        .iter()
        .map(|value| value.elapsed_millis)
        .collect();
    assert_near(
        "performance.all_median_millis",
        metrics.performance.all_median_millis,
        stage_3c_median_millis(&all_elapsed),
        STAGE_3C_DERIVED_MILLIS_TOLERANCE,
    );
    assert_eq!(
        metrics.performance.all_p95_millis,
        stage_3c_nearest_rank(&all_elapsed, 95, 100)
    );
    assert_eq!(metrics.performance.bands.len(), 5);
    for band_metrics in &metrics.performance.bands {
        let expected = expected_band_values
            .remove(band_metrics.band.as_str())
            .unwrap_or_else(|| panic!("unexpected band {}", band_metrics.band));
        assert_eq!(band_metrics.values, expected);
        assert_eq!(band_metrics.values.len(), 30);
        let elapsed: Vec<_> = band_metrics
            .values
            .iter()
            .map(|value| value.elapsed_millis)
            .collect();
        assert_near(
            "performance.band.median_millis",
            band_metrics.median_millis,
            stage_3c_median_millis(&elapsed),
            STAGE_3C_DERIVED_MILLIS_TOLERANCE,
        );
        assert_eq!(
            band_metrics.p95_millis,
            stage_3c_nearest_rank(&elapsed, 95, 100)
        );
    }
    assert!(expected_band_values.is_empty());
    assert_eq!(metrics.performance.corpus_rounds.len(), 5);
    let corpus_elapsed: Vec<_> = metrics
        .performance
        .corpus_rounds
        .iter()
        .enumerate()
        .map(|(index, round)| {
            assert_eq!(round.repetition, index + 1);
            let repetition = index + 1;
            let expected_product_elapsed: u64 = metrics
                .performance
                .all_values
                .iter()
                .filter(|value| value.repetition == repetition)
                .map(|value| value.elapsed_millis)
                .sum();
            assert_eq!(
                round.product_elapsed_millis, expected_product_elapsed,
                "round {repetition}: product elapsed must be the sum of 30 case measurements"
            );
            assert!(
                round.collector_wall_elapsed_millis >= round.product_elapsed_millis,
                "round {repetition}: collector wall time cannot be shorter than product time"
            );
            round.product_elapsed_millis
        })
        .collect();
    assert_near(
        "performance.corpus_median_millis",
        metrics.performance.corpus_median_millis,
        stage_3c_median_millis(&corpus_elapsed),
        STAGE_3C_DERIVED_MILLIS_TOLERANCE,
    );
    assert_eq!(
        metrics.performance.corpus_p95_millis,
        stage_3c_nearest_rank(&corpus_elapsed, 95, 100)
    );

    assert_eq!(
        metrics.outliers.method,
        "tukey_per_case_nearest_rank_1_5_iqr"
    );
    assert_eq!(metrics.outliers.scope, "each_case_five_release_values");
    assert!(!metrics.outliers.excluded_from_aggregates);
    let mut expected_outliers = Vec::new();
    for case_metrics in &metrics.cases {
        let elapsed: Vec<_> = case_metrics
            .performance
            .runs
            .iter()
            .map(|run| run.elapsed_millis)
            .collect();
        let q1 = stage_3c_nearest_rank(&elapsed, 25, 100) as f64;
        let q3 = stage_3c_nearest_rank(&elapsed, 75, 100) as f64;
        let iqr = q3 - q1;
        let lower_fence_millis = q1 - 1.5 * iqr;
        let upper_fence_millis = q3 + 1.5 * iqr;
        for run in &case_metrics.performance.runs {
            let value = run.elapsed_millis as f64;
            if value < lower_fence_millis || value > upper_fence_millis {
                expected_outliers.push(Stage3cMetricsOutlier {
                    case_id: case_metrics.case_id.clone(),
                    repetition: run.repetition,
                    elapsed_millis: run.elapsed_millis,
                    lower_fence_millis,
                    upper_fence_millis,
                });
            }
        }
    }
    assert_eq!(metrics.outliers.values.len(), expected_outliers.len());
    for (actual, expected) in metrics.outliers.values.iter().zip(&expected_outliers) {
        assert_eq!(actual.case_id, expected.case_id);
        assert_eq!(actual.repetition, expected.repetition);
        assert_eq!(actual.elapsed_millis, expected.elapsed_millis);
        assert_near(
            "outlier.lower_fence_millis",
            actual.lower_fence_millis,
            expected.lower_fence_millis,
            STAGE_3C_DERIVED_MILLIS_TOLERANCE,
        );
        assert_near(
            "outlier.upper_fence_millis",
            actual.upper_fence_millis,
            expected.upper_fence_millis,
            STAGE_3C_DERIVED_MILLIS_TOLERANCE,
        );
    }

    let gate = &metrics.gate_proposal;
    assert_eq!(gate.source, "sum_of_case_elapsed_millis_p95");
    assert_near(
        "gate.performance_baseline_fraction",
        gate.performance_baseline_fraction,
        0.8,
        f64::EPSILON,
    );
    assert_eq!(
        gate.reference_p95_millis,
        metrics.performance.corpus_p95_millis
    );
    let (raw_gate_millis, gate_seconds, gate_millis) =
        stage_3c_gate_from_p95(gate.reference_p95_millis);
    assert_eq!(gate.raw_gate_millis, raw_gate_millis);
    assert_eq!(gate.proposed_gate_seconds, gate_seconds);
    assert_eq!(gate.proposed_gate_millis, gate_millis);
    assert_eq!(gate.case_source, "maximum_case_p95_millis");
    let case_p95 = case_p95_values
        .into_iter()
        .max()
        .expect("30 case P95 values");
    assert_eq!(gate.case_reference_p95_millis, case_p95);
    let (case_raw_gate_millis, case_gate_seconds, case_gate_millis) =
        stage_3c_gate_from_p95(case_p95);
    assert_eq!(gate.case_raw_gate_millis, case_raw_gate_millis);
    assert_eq!(gate.proposed_case_gate_seconds, case_gate_seconds);
    assert_eq!(gate.proposed_case_gate_millis, case_gate_millis);
    assert!(!gate.enforced);
    assert_eq!(gate.status, "awaiting_coordinator");
    stage_3c_assert_target_summary(&metrics.target_summary, manifest);
    assert!(
        determinism_mismatch_cases.is_empty() && determinism_recorded_mismatch_cases.is_empty(),
        "3-C fingerprint acceptance failed: determinism={determinism_mismatch_cases:?}; \
         determinism_vs_recorded_current={determinism_recorded_mismatch_cases:?}"
    );
}

#[test]
fn stage_3c_release_metrics_fixture_is_strict_read_only_evidence() {
    let (manifest_before, manifest) = load_manifest().expect("manifest");
    let immutable = stage_3c_immutable_files(&manifest);
    let metrics_path = stage_3c_metrics_path();
    let metrics_bytes = immutable
        .get(&metrics_path)
        .expect("3-C metrics bytes were snapshotted");
    let metrics: Stage3cMetricsFixture =
        serde_json::from_slice(metrics_bytes).expect("3-C metrics schema");
    let verification = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stage_3c_assert_metrics_fixture(&metrics, &manifest, &immutable);
    }));
    for (path, before) in immutable {
        let after = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            after,
            before,
            "3-C read-only test changed {}",
            path.display()
        );
    }
    assert_eq!(
        fs::read(manifest_path()).expect("manifest after"),
        manifest_before,
        "3-C read-only test changed manifest"
    );
    if let Err(payload) = verification {
        std::panic::resume_unwind(payload);
    }
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
