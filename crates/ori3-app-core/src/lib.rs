//! Tauriとブラウザの両方から利用する、実行環境に依存しないアプリケーション境界。
//!
//! 18個の関数は、デスクトップ版のTauri commandからhost固有引数
//! (`State` / `AppHandle`)だけを取り除いた契約である。文書状態はcoreが所有し、
//! 全18関数がデスクトップ版と同じ状態遷移と返答を持つ。ブラウザ固有のI/Oは、
//! 公開18命令へ渡す前後にprivate host commandで準備・確定する。

use std::collections::{HashMap, HashSet};

use ori3_cp::{Face, extract_faces, local_violations, validate};
use ori3_export::fold::{FoldExport, FoldImport, FoldIssue};
use ori3_export::{
    CpSvgOptions, cp_png, cp_svg, diagram_pdf, diagram_svg_pages, document_to_fold,
    fold_to_document, parse_fold_1_2, write_fold_1_2,
};
use ori3_model::{
    CreasePattern, DisplaySettings, Document, Driver, EdgeId, EdgeKind, EditOp, FaceId,
    FinishSoftSettings, FoldStep, FoldTargetInfo, FoldTargetStatus, FoldTargetTopAction, Frame3D,
    MAX_GRID_DIVISIONS, MIN_GRID_DIVISIONS, Paper, SCHEMA_VERSION, SavedDocument, SeqOp,
    StepCreases, StepId, TechniqueKind, VertexId,
};
use ori3_propose::{
    CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, LeafSite, Packing,
    PoseScan, SearchAbort, SearchBudget, SearchCancellation, SearchControl, SearchWatchdog,
    Skeleton, TipSite, VerifiedPlan, body_on_paper, generate, pack,
    search_to_completion_with_control, verify_search_completion,
};
use ori3_soft::{SoftMesh, SoftSettings};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

include!(concat!(env!("OUT_DIR"), "/desktop_contract_check.rs"));

/// desktopのDocumentStoreと同じundo履歴上限。
const MAX_UNDO: usize = 100;

const FOLD_ALL_FLAT_FOLD_WARNING: &str =
    "平らにたためない折り目の集まりがあります。表示できた形を返しています";

/// フロントエンドが利用するバックエンドコマンド名。
///
/// 固定長と後段の関数ポインター検査により、関数を1件でも消すと組み立てに失敗する。
pub const BACKEND_COMMAND_NAMES: [&str; 18] = [
    "document_new",
    "document_open",
    "document_save",
    "edit_apply",
    "edit_apply_batch",
    "edit_undo",
    "edit_redo",
    "sequence_apply",
    "sequence_replay",
    "pose_solve",
    "fold_all_preview",
    "recovery_check",
    "recovery_restore",
    "proposal_generate",
    "proposal_progress",
    "proposal_control",
    "proposal_apply",
    "document_export",
];

/// ブラウザhostとRust coreの間だけで使う、外部18命令へは露出しないI/O準備口。
const WEB_HOST_COMMAND_NAMES: [&str; 10] = [
    "__web_document_open_source",
    "__web_document_save_prepare",
    "__web_document_save_abort",
    "__web_document_export_prepare",
    "__web_recovery_set_choices",
    "__web_recovery_restore_source",
    "__web_recovery_snapshot",
    "__web_proposal_prepare",
    "__web_proposal_generate_candidate",
    "__web_proposal_verify_candidate",
];

const MAX_SAFE_CANDIDATE_ID: u64 = 9_007_199_254_740_991;

/// デスクトップ側の`store::DocumentView`と同じwire形。
#[derive(Serialize)]
pub struct DocumentView {
    pub doc: Document,
    pub step_creases: Vec<StepCreases>,
    pub fold_issues: Vec<FoldIssue>,
    pub faces: Vec<Face>,
    pub warnings: Vec<String>,
    pub violations: Vec<VertexId>,
    pub flat_fold_violations: Vec<VertexId>,
    pub frame: Option<Frame3D>,
    pub skipped: Vec<StepId>,
    pub suspect_hinges: Vec<EdgeId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub self_intersection_pairs: Vec<(FaceId, FaceId)>,
    pub contact_detected: bool,
    pub sequence_targets: Vec<Driver>,
    pub angles: HashMap<EdgeId, f64>,
    pub relaxations: Vec<ori3_rigid::AngleRelaxation>,
    pub closure_rms: Option<f64>,
    pub best_effort: bool,
    pub converged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fold_through_proposal: Option<ori3_layers::FoldThroughProposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fold_target_info: Option<FoldTargetInfo>,
}

/// 通常の連続操作と、Documentから再導出する確定操作を区別する。
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub enum PoseSolveMode {
    #[default]
    Follow,
    Canonical,
}

/// `pose_solve`の単一wire引数。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoseSolveRequest {
    hard: Vec<Driver>,
    preferred: Option<Vec<Driver>>,
    soft: Option<SoftSettings>,
    warm_seed: Option<Vec<Driver>>,
    up_to: usize,
    t: f64,
    mode: Option<PoseSolveMode>,
}

/// デスクトップ側の`commands::PoseOutcome`と同じwire形。
#[derive(Serialize)]
pub struct PoseOutcome {
    #[serde(flatten)]
    pub result: ori3_rigid::SolveResult,
    pub soft: Option<SoftMesh>,
    pub suspect_hinges: Vec<EdgeId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub self_intersection_pairs: Vec<(FaceId, FaceId)>,
    pub contact_detected: bool,
    pub flat_fold_violations: Vec<VertexId>,
}

/// 手順を持たない一斉折りでは、物理的な上下を返せないことを示す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldAllLayerOrder {
    UnavailableWithoutSequence,
}

/// デスクトップ側の`commands::FoldAllPreviewOutcome`と同じwire形。
#[derive(Serialize)]
pub struct FoldAllPreviewOutcome {
    #[serde(flatten)]
    pub result: ori3_rigid::SolveResult,
    pub requested_percent: f64,
    pub requested_angles: Vec<Driver>,
    pub next_warm_seed: Vec<Driver>,
    pub suspect_hinges: Vec<EdgeId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub self_intersection_pairs: Vec<(FaceId, FaceId)>,
    pub contact_detected: bool,
    pub flat_fold_violations: Vec<VertexId>,
    pub layer_order: FoldAllLayerOrder,
}

/// デスクトップ側の`commands::ReplayOutcome`と同じwire形。
#[derive(Serialize)]
pub struct ReplayOutcome {
    #[serde(flatten)]
    pub result: ori3_layers::ReplayResult,
    pub soft: Option<SoftMesh>,
    pub sequence_targets: Vec<Driver>,
    pub angles: HashMap<EdgeId, f64>,
    pub relaxations: Vec<ori3_rigid::AngleRelaxation>,
    pub closure_rms: Option<f64>,
    pub best_effort: bool,
    pub converged: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub self_intersection_pairs: Vec<(FaceId, FaceId)>,
    pub contact_detected: bool,
    pub flat_fold_violations: Vec<VertexId>,
}

/// 現HEADの`autosave::RecoveryInfo`と同じwire形。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RecoveryInfo {
    pub autosave_path: String,
    pub document_path: Option<String>,
    pub saved_at_ms: Option<u64>,
    pub candidate_id: u64,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub step_count: Option<usize>,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RecoveryChoices {
    pub choices: Vec<RecoveryInfo>,
    pub overflow_count: usize,
}

/// 提案された折り方の共通wire部分。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProposalFoldPlanDetails {
    pub steps: Vec<FoldStep>,
    pub cp: CreasePattern,
    pub planned: usize,
    pub checked: usize,
}

/// 提案された折り方の判別可能なwire形。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProposalFoldPlan {
    #[serde(flatten)]
    state: ProposalFoldPlanState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProposalFoldPlanState {
    CheckedToFinish {
        #[serde(flatten)]
        details: ProposalFoldPlanDetails,
    },
    Partial {
        #[serde(flatten)]
        details: ProposalFoldPlanDetails,
    },
}

impl ProposalFoldPlan {
    fn from_verified(verified: VerifiedPlan, details: ProposalFoldPlanDetails) -> Self {
        match verified {
            VerifiedPlan::CheckedToFinish(checked)
                if details.checked == details.planned
                    && details.planned == checked.report().requested =>
            {
                Self {
                    state: ProposalFoldPlanState::CheckedToFinish { details },
                }
            }
            VerifiedPlan::CheckedToFinish(_) | VerifiedPlan::Partial(_) => Self {
                state: ProposalFoldPlanState::Partial { details },
            },
        }
    }
}

/// `proposal_generate`が返す候補1件のwire形。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProposalCandidate {
    pub cp: CreasePattern,
    pub scale: f64,
    pub violations: usize,
    pub warnings: Vec<String>,
    pub sites: Vec<LeafSite>,
    pub fold_plan: Option<ProposalFoldPlan>,
}

/// 画面が作る不透明な提案job ID。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProposalJobId(String);

impl ProposalJobId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 提案jobの単調な進行段階。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ProposalPhase {
    Queued,
    Generating,
    Verifying,
    Finished,
    Cancelled,
    Failed,
}

/// `proposal_progress`の同一時点snapshotのwire形。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProposalProgressSnapshot {
    pub job_id: ProposalJobId,
    pub done: usize,
    pub total: usize,
    pub phase: ProposalPhase,
}

/// `proposal_control`へ渡す閉じた操作。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ProposalControl {
    Cancel { job_id: ProposalJobId },
}

/// `proposal_generate`のjob別wire戻り値。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalJobResult {
    pub job_id: ProposalJobId,
    pub candidates: Vec<ProposalCandidate>,
}

/// desktopの候補数・探索予算と同じ値。WASMでは候補を専用Worker内で順番に処理する。
const PROPOSAL_PACK_STARTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProposalPlanBudget {
    deterministic: SearchBudget,
    watchdog: SearchWatchdog,
}

const PROPOSAL_PLAN_BUDGET: ProposalPlanBudget = ProposalPlanBudget {
    deterministic: SearchBudget {
        max_states: 2,
        branch: 2,
        max_depth: SearchBudget::DEFAULT.max_depth,
        rank_scan: SearchBudget::DEFAULT.rank_scan,
        scan: SearchBudget::DEFAULT.scan,
    },
    watchdog: SearchWatchdog { max_millis: 30_000 },
};

const PROPOSAL_PLAN_REBUILD_SCAN: PoseScan = PoseScan { steps: 0 };

#[derive(Clone, Debug, PartialEq, Serialize)]
struct WebProposalPreparation {
    paper_w: f64,
    paper_h: f64,
    packings: Vec<Packing>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct WebProposalCandidateGeneration {
    candidate: Option<ProposalCandidate>,
    error: Option<String>,
}

/// `document_export`の書き出し種別。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportKind {
    CpSvg,
    CpPng,
    DiagramPdf,
    DiagramSvg,
    FoldJson,
}

/// `document_export`の細かい指定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    pub include_aux: bool,
    pub png_long_side: i64,
}

/// Web hostがファイルへ書く前にRust coreで確定した保存内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentSavePreparation {
    pub path: String,
    pub content: String,
}

/// Web hostへ渡す1ファイル分の書き出し内容。
///
/// `first_cell` / `last_cell` は利用者向けの1始まりのコマ番号であり、表紙は`None`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentExportFile {
    pub suffix: String,
    pub content_type: String,
    pub content_base64: String,
    pub page_number: Option<usize>,
    pub first_cell: Option<usize>,
    pub last_cell: Option<usize>,
}

/// Web hostが保存先へ配送する、全ファイルを先に作り切った結果。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DocumentExportPreparation {
    pub files: Vec<DocumentExportFile>,
    pub fold_issues: Vec<FoldIssue>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WebRecoverySnapshot {
    pub doc: Document,
    pub step_creases: Vec<StepCreases>,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingDocumentOpen {
    path: String,
    source: String,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingDocumentSave {
    path: String,
    saved: SavedDocument,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingRecoverySource {
    candidate_id: u64,
    document_path: Option<String>,
    source: String,
}

/// undo/redoで行き来する、hostに依存しない作品状態。
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    doc: Document,
    step_creases: Vec<StepCreases>,
}

/// 3D画面で実際に当たった点から作る、作品へ保存しない一時wire入力。
#[derive(Clone, Debug, Deserialize)]
pub struct SpatialFoldSpec {
    pub from: [f64; 3],
    pub to: [f64; 3],
    #[serde(alias = "grabFace")]
    pub grab_face: FaceId,
}

/// document状態と18コマンドの処理を所有する、host-neutralなアプリケーションcore。
///
/// pathにはWeb側で解決する不透明なfile tokenを保持できる。Rust core自身は
/// OSのファイルシステムへ触れず、外部I/Oの成功後にだけ状態を確定する。
#[derive(Clone, Debug, PartialEq)]
pub struct Ori3AppCore {
    doc: Document,
    step_creases: Vec<StepCreases>,
    faces: Vec<Face>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    dirty: bool,
    path: Option<String>,
    pose_angles: Option<HashMap<EdgeId, f64>>,
    pending_open: Option<PendingDocumentOpen>,
    pending_save: Option<PendingDocumentSave>,
    staged_recovery_choices: Option<Option<RecoveryChoices>>,
    pending_recovery_source: Option<PendingRecoverySource>,
}

impl Default for Ori3AppCore {
    /// desktopのDocumentStore::defaultと同じ150mm正方形の起動状態。
    fn default() -> Self {
        let doc = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        });
        Self {
            faces: extract_faces(&doc.cp),
            doc,
            step_creases: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
            path: None,
            pose_angles: None,
            pending_open: None,
            pending_save: None,
            staged_recovery_choices: None,
            pending_recovery_source: None,
        }
    }
}

impl Ori3AppCore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn document_new(&mut self, paper: Paper) -> Result<DocumentView, String> {
        validate_paper_dimensions(&paper)?;

        let doc = Document::new(paper);
        let view = build_initial_document_view(&doc);
        self.doc = doc;
        self.step_creases.clear();
        self.faces.clone_from(&view.faces);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = None;
        self.pose_angles = None;
        self.pending_open = None;
        self.pending_save = None;
        self.staged_recovery_choices = None;
        self.pending_recovery_source = None;
        Ok(view)
    }

    pub fn document_open(&mut self, path: String) -> Result<DocumentView, String> {
        let pending = self.pending_open.take().ok_or_else(|| {
            "開くファイルの内容が準備されていません。もう一度ファイルを選んでください。".to_owned()
        })?;
        if pending.path != path {
            return Err(
                "選んだファイルと読み取った内容が一致しません。もう一度ファイルを選んでください。"
                    .to_owned(),
            );
        }
        let view = self.open_document_source(path, pending.source)?;
        self.staged_recovery_choices = None;
        self.pending_recovery_source = None;
        Ok(view)
    }

    pub fn document_save(&mut self, path: Option<String>) -> Result<(), String> {
        let target = match path {
            Some(path) => path,
            None => self
                .path
                .clone()
                .ok_or_else(|| "保存先が指定されていません".to_owned())?,
        };
        let pending = self.pending_save.take().ok_or_else(|| {
            "保存する内容が準備されていません。もう一度保存してください。".to_owned()
        })?;
        if pending.path != target || pending.saved != self.saved_document() {
            return Err(
                "保存内容を準備した後に作品が変わりました。もう一度保存してください。".to_owned(),
            );
        }
        self.path = Some(target);
        self.dirty = false;
        self.staged_recovery_choices = None;
        self.pending_recovery_source = None;
        Ok(())
    }

    fn stage_document_open(&mut self, path: String, source: String) {
        self.pending_open = Some(PendingDocumentOpen { path, source });
    }

    fn stage_recovery_choices(&mut self, choices: Option<RecoveryChoices>) -> Result<(), String> {
        if let Some(choices) = &choices {
            validate_recovery_choices(choices)?;
        }
        self.staged_recovery_choices = Some(choices);
        Ok(())
    }

    fn stage_recovery_source(
        &mut self,
        candidate_id: u64,
        document_path: Option<String>,
        source: String,
    ) -> Result<(), String> {
        validate_candidate_id(candidate_id)?;
        self.pending_recovery_source = Some(PendingRecoverySource {
            candidate_id,
            document_path,
            source,
        });
        Ok(())
    }

    fn web_recovery_snapshot(&self) -> Option<WebRecoverySnapshot> {
        if !self.dirty {
            return None;
        }
        let saved = self.saved_document();
        Some(WebRecoverySnapshot {
            doc: saved.document,
            step_creases: saved.step_creases,
        })
    }

    fn prepare_document_save(
        &mut self,
        path: Option<String>,
    ) -> Result<DocumentSavePreparation, String> {
        let target = match path {
            Some(path) => path,
            None => self
                .path
                .clone()
                .ok_or_else(|| "保存先が指定されていません".to_owned())?,
        };
        let saved = self.saved_document();
        let content = serde_json::to_string_pretty(&saved)
            .map_err(|error| format!("保存データの作成に失敗しました: {error}"))?;
        self.pending_save = Some(PendingDocumentSave {
            path: target.clone(),
            saved,
        });
        Ok(DocumentSavePreparation {
            path: target,
            content,
        })
    }

    fn abort_document_save(&mut self) {
        self.pending_save = None;
    }

    fn saved_document(&self) -> SavedDocument {
        SavedDocument {
            document: self.doc.clone(),
            step_creases: retain_existing_steps(&self.doc, &self.step_creases),
        }
    }

    fn open_document_source(
        &mut self,
        path: String,
        source: String,
    ) -> Result<DocumentView, String> {
        if is_fold_path(&path) {
            let file = parse_fold_1_2(&source).map_err(|_| {
                "ほかの折り紙ソフトのファイルを読み取れませんでした。ファイルの内容を確認してください。"
                    .to_owned()
            })?;
            let FoldImport { document, warnings } = fold_to_document(&file)
                .map_err(|_| "このファイルには、ORIGAMI3で扱えない内容があります。".to_owned())?;
            let step_creases = Vec::new();
            let mut view = build_document_view(&document, &step_creases, Vec::new());
            view.fold_issues = warnings;
            let view = self.commit_prebuilt(document, step_creases, view);
            self.dirty = true;
            self.path = None;
            self.pose_angles = None;
            self.pending_save = None;
            return Ok(self.finish_document_view(view));
        }

        let saved = parse_saved_document(&source)?;
        let view = build_document_view(&saved.document, &saved.step_creases, Vec::new());
        self.doc = saved.document;
        self.step_creases = saved.step_creases;
        self.faces.clone_from(&view.faces);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = Some(path);
        self.pose_angles = None;
        self.pending_save = None;
        Ok(self.finish_document_view(view))
    }

    pub fn edit_apply(&mut self, op: EditOp) -> Result<DocumentView, String> {
        self.apply_edits(vec![op])
    }

    pub fn edit_apply_batch(&mut self, ops: Vec<EditOp>) -> Result<DocumentView, String> {
        self.apply_edits(ops)
    }

    pub fn edit_undo(&mut self) -> Result<DocumentView, String> {
        let prev = self
            .undo_stack
            .last()
            .ok_or_else(|| "これ以上元に戻せません".to_owned())?;
        let view = build_document_view(&prev.doc, &prev.step_creases, Vec::new());
        let prev = self.undo_stack.pop().expect("直前にlastで確認済み");
        self.redo_stack.push(Snapshot {
            doc: std::mem::replace(&mut self.doc, prev.doc),
            step_creases: std::mem::replace(&mut self.step_creases, prev.step_creases),
        });
        self.faces.clone_from(&view.faces);
        self.dirty = true;
        Ok(self.finish_document_view(view))
    }

    pub fn edit_redo(&mut self) -> Result<DocumentView, String> {
        let next = self
            .redo_stack
            .last()
            .ok_or_else(|| "これ以上やり直せません".to_owned())?;
        let view = build_document_view(&next.doc, &next.step_creases, Vec::new());
        let next = self.redo_stack.pop().expect("直前にlastで確認済み");
        self.undo_stack.push(Snapshot {
            doc: std::mem::replace(&mut self.doc, next.doc),
            step_creases: std::mem::replace(&mut self.step_creases, next.step_creases),
        });
        self.faces.clone_from(&view.faces);
        self.dirty = true;
        Ok(self.finish_document_view(view))
    }

    /// 複数の編集を利用者の1操作として候補cloneへ適用し、最後に1回だけ確定する。
    fn apply_edits(&mut self, ops: Vec<EditOp>) -> Result<DocumentView, String> {
        if ops.is_empty() {
            return Err("編集する内容がありません".to_owned());
        }
        let replaced_crease_pattern = ops
            .iter()
            .any(|op| matches!(op, EditOp::ReplaceCreasePattern { .. }));
        let mut doc = self.doc.clone();
        let mut warnings = Vec::new();
        for op in ops {
            edit_document(&mut doc, op, &mut warnings)?;
        }
        let step_creases = self.step_creases.clone();
        let view = self.commit(doc, step_creases, warnings);
        if replaced_crease_pattern {
            self.pose_angles = None;
        }
        Ok(self.finish_document_view(view))
    }

    /// 導出を候補値から先に完了し、変更がある場合だけundoへ積んで確定する。
    fn commit(
        &mut self,
        doc: Document,
        step_creases: Vec<StepCreases>,
        warnings: Vec<String>,
    ) -> DocumentView {
        let view = build_document_view(&doc, &step_creases, warnings);
        self.commit_prebuilt(doc, step_creases, view)
    }

    /// 候補Documentから全導出済みのviewを、失敗点を残さず1回で確定する。
    fn commit_prebuilt(
        &mut self,
        doc: Document,
        step_creases: Vec<StepCreases>,
        view: DocumentView,
    ) -> DocumentView {
        if doc != self.doc || step_creases != self.step_creases {
            if self.undo_stack.len() >= MAX_UNDO {
                self.undo_stack.remove(0);
            }
            let next = Snapshot { doc, step_creases };
            let prev = Snapshot {
                doc: std::mem::replace(&mut self.doc, next.doc),
                step_creases: std::mem::replace(&mut self.step_creases, next.step_creases),
            };
            self.undo_stack.push(prev);
            self.faces.clone_from(&view.faces);
            self.redo_stack.clear();
            self.dirty = true;
            self.pending_save = None;
        }
        view
    }

    /// desktop commands::view_commandと同じ再生・warm start保存の後処理。
    fn finish_document_view(&mut self, mut view: DocumentView) -> DocumentView {
        attach_replay(&mut view);
        if view.frame.is_some() && view.angles.values().all(|angle| angle.is_finite()) {
            self.pose_angles = Some(view.angles.clone());
        }
        view
    }

    pub fn sequence_apply(&mut self, op: Value) -> Result<DocumentView, String> {
        let (mut operation, spatial) = parse_sequence_operation(op)?;
        let is_move_step = matches!(&operation, SeqOp::MoveStep { .. });
        let document = self.doc.clone();
        record_finish_soft(&mut operation, &document.display);
        let mut view = self.apply_seq_with_spatial(operation, spatial)?;
        let move_step_noop = is_move_step && view.doc == document;
        if view.frame.is_none() {
            attach_replay(&mut view);
        }
        if !move_step_noop
            && view.frame.is_some()
            && view.angles.values().all(|angle| angle.is_finite())
        {
            self.pose_angles = Some(view.angles.clone());
        }
        Ok(view)
    }

    pub fn sequence_replay(
        &mut self,
        up_to: usize,
        t: f64,
        soft: Option<SoftSettings>,
    ) -> Result<ReplayOutcome, String> {
        let doc = self.doc.clone();
        let faces = self.faces.clone();
        let soft = display_soft_settings(&doc, up_to, t, soft);
        let mut result = ori3_layers::replay_with_faces(&doc, &faces, up_to, t);
        let saved_order = ori3_layers::saved_layer_order_at(&doc, &faces, up_to, t);
        let canonical_order = replay_surface_rank_order(&result);
        let completed = !t.is_finite() || t >= 1.0;
        let mut penetration_warnings: Vec<&'static str> = Vec::new();
        if completed && saved_order.is_none() {
            let warning = apply_layer_order_display_settings(
                &doc.cp,
                &faces,
                &mut result.frame,
                canonical_order.as_deref(),
                doc.display.overlap_prevention_enabled,
                doc.display.penetration_prevention_enabled,
            );
            if let Some(warning) = warning {
                penetration_warnings.push(warning);
            }
        }
        let intersections = if doc.display.penetration_prevention_enabled {
            ori3_rigid::self_intersection_pairs(&result.frame)
        } else {
            Vec::new()
        };
        let contact_detected = !intersections.is_empty();
        let overlap_settings = ori3_soft::OverlapSettings {
            enabled: doc.display.overlap_prevention_enabled,
            ..Default::default()
        };
        let _ = prevent_replay_overlap_if_authoritative(
            &doc.cp,
            &faces,
            &mut result,
            &overlap_settings,
        );
        result.suspect_hinges = ori3_rigid::suspect_hinges_for_intersections(
            &doc.cp,
            &faces,
            &intersections,
            &result.driver_hinges,
        );
        penetration_warnings.extend(add_penetration_warning_for_intersections(
            &doc.cp,
            &faces,
            &mut result.frame,
            false,
            &intersections,
        ));
        for warning in penetration_warnings {
            if !result.warnings.iter().any(|existing| existing == warning) {
                result.warnings.push(warning.to_owned());
            }
        }
        filter_penetration_warnings(
            &mut result.warnings,
            doc.display.penetration_prevention_enabled,
        );
        filter_penetration_warnings(
            &mut result.frame.warnings,
            doc.display.penetration_prevention_enabled,
        );
        let mesh = soft_mesh(
            &doc.cp,
            &faces,
            &result.frame,
            canonical_order.is_some(),
            soft.as_ref(),
        );
        let flat_fold_violations = replay_flat_fold_notice_violations(
            &doc.cp,
            &result.sequence_targets,
            &result.hinge_angles,
            &intersections,
        );
        let angles = result.hinge_angles.clone();
        let sequence_targets = result.sequence_targets.clone();
        let relaxations = result.relaxations.clone();
        let closure_rms = result.closure_rms.is_finite().then_some(result.closure_rms);
        let best_effort = result.best_effort;
        let converged = result.converged;
        if angles.values().all(|angle| angle.is_finite()) {
            self.pose_angles = Some(angles.clone());
        }
        Ok(ReplayOutcome {
            result,
            soft: mesh,
            sequence_targets,
            angles,
            relaxations,
            closure_rms,
            best_effort,
            converged,
            self_intersection_pairs: intersections,
            contact_detected,
            flat_fold_violations,
        })
    }

    fn apply_seq_with_spatial(
        &mut self,
        op: SeqOp,
        spatial: Option<SpatialFoldSpec>,
    ) -> Result<DocumentView, String> {
        let mut doc = self.doc.clone();
        let mut step_creases = self.step_creases.clone();
        let mut warnings = Vec::new();
        let mut fold_through_proposal = None;
        match op {
            SeqOp::PushStep { step } => {
                record_frontend_step(&mut step_creases, &step);
                doc.sequence.push(step);
            }
            SeqOp::InsertStep { index, step } => {
                if index > doc.sequence.len() {
                    return Err(format!("挿入位置 {index} が手順の数を超えています"));
                }
                record_frontend_step(&mut step_creases, &step);
                doc.sequence.insert(index, step);
            }
            SeqOp::RemoveStep { id } => {
                let before = doc.sequence.len();
                doc.sequence.retain(|step| step.id != id);
                if doc.sequence.len() == before {
                    return Err(format!("手順ID {id} が見つかりません"));
                }
            }
            SeqOp::MoveStep { id, to_index } => {
                let mut seen = HashSet::with_capacity(doc.sequence.len());
                if doc.sequence.iter().any(|step| !seen.insert(step.id)) {
                    return Err("同じ折り手順が二重に入っています".to_owned());
                }
                let Some(from) = doc.sequence.iter().position(|step| step.id == id) else {
                    return Err(format!("手順ID {id} が見つかりません"));
                };
                if to_index >= doc.sequence.len() {
                    return Err(format!("移動先 {to_index} が手順の数を超えています"));
                }
                if from != to_index {
                    let moved = doc.sequence.remove(from);
                    doc.sequence.insert(to_index, moved);
                }
                let view = build_move_step_view(&doc, &step_creases);
                if from == to_index {
                    return Ok(view);
                }
                return Ok(self.commit_prebuilt(doc, step_creases, view));
            }
            SeqOp::UpdateStep { step } => {
                let Some(slot) = doc
                    .sequence
                    .iter_mut()
                    .find(|existing| existing.id == step.id)
                else {
                    return Err(format!("手順ID {} が見つかりません", step.id));
                };
                *slot = step;
            }
            SeqOp::FoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                alignment,
                accept_additional_crease,
                pose_before,
            } => {
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                if target_layers.is_some() && target_pleat_count.is_some() {
                    return Err("折るひだの枚数と個別の紙を同時には指定できません".to_string());
                }
                if target_pleat_count.is_some() && spatial.is_some() {
                    return Err(
                        "折るひだの枚数と3D上のつかみ位置を同時には指定できません".to_string()
                    );
                }
                let target_layers = if let Some(count) = target_pleat_count {
                    Some(fold_target_faces_at(
                        &doc,
                        &self.faces,
                        up_to,
                        line,
                        keep_side_point,
                        pose_before.as_ref(),
                        count,
                    )?)
                } else {
                    target_layers
                };
                if pose_before.is_some() && spatial.is_some() {
                    return Err(
                        "折った形の再現と3D上のつかみ位置を、同時には指定できません".to_owned()
                    );
                }
                if let Some(pose_input) = pose_before {
                    let pose = ori3_layers::replay::canonical_flat_pose_at(
                        &doc,
                        &self.faces,
                        up_to,
                        &pose_input,
                    )?;
                    let mut pose_step = pose.step;
                    pose_step.id = next_step_id(&doc, &step_creases);
                    record_frontend_step(&mut step_creases, &pose_step);
                    doc.sequence.insert(up_to, pose_step);

                    let mut cp = doc.cp.clone();
                    let result = ori3_layers::fold_through_with_additional_crease(
                        &mut cp,
                        &self.faces,
                        &pose.state,
                        &ori3_layers::FoldThroughInput {
                            line,
                            keep_side_point,
                            target_layers,
                            direction,
                        },
                        accept_additional_crease,
                    )?;
                    let mut step = result.step;
                    step.id = next_step_id(&doc, &step_creases);
                    step.alignment = alignment;
                    let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                    record_step_creases(&mut step_creases, step.id, lines);
                    doc.cp = cp;
                    doc.sequence.insert(up_to + 1, step);
                    warnings = pose.warnings;
                    warnings.append(&mut insert_warnings);
                    warnings.extend(result.warnings);
                } else {
                    let current = spatial
                        .as_ref()
                        .map(|_| ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0));
                    if let Some(current) = current
                        .as_ref()
                        .filter(|current| frame_is_nonflat(&current.frame))
                    {
                        let input = spatial_fold_input(
                            spatial.as_ref(),
                            &current.frame,
                            &self.faces,
                            line,
                            keep_side_point,
                            target_layers.as_deref(),
                            direction,
                        );
                        let mut result =
                            ori3_layers::fold_from_plane_3d(&doc, &self.faces, up_to, &input);
                        if let Some(mut step) = result.step.take() {
                            step.id = next_step_id(&doc, &step_creases);
                            step.alignment = alignment;
                            let lines =
                                added_crease_lines(&doc.cp, &result.cp, &result.added_edges);
                            record_step_creases(&mut step_creases, step.id, lines);
                            doc.cp = result.cp;
                            doc.sequence.insert(up_to, step);
                        }
                        warnings.append(&mut insert_warnings);
                        warnings.append(&mut result.warnings);
                    } else {
                        match ori3_layers::flat_state_at(&doc, &self.faces, up_to) {
                            Ok((state, state_warnings)) => {
                                let mut cp = doc.cp.clone();
                                let result = ori3_layers::fold_through_with_additional_crease(
                                    &mut cp,
                                    &self.faces,
                                    &state,
                                    &ori3_layers::FoldThroughInput {
                                        line,
                                        keep_side_point,
                                        target_layers,
                                        direction,
                                    },
                                    accept_additional_crease,
                                )?;
                                let mut step = result.step;
                                step.id = next_step_id(&doc, &step_creases);
                                step.alignment = alignment;
                                let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                                record_step_creases(&mut step_creases, step.id, lines);
                                doc.cp = cp;
                                doc.sequence.insert(up_to, step);
                                warnings = state_warnings;
                                warnings.append(&mut insert_warnings);
                                warnings.extend(result.warnings);
                            }
                            Err(flat_error) => {
                                let current = current.unwrap_or_else(|| {
                                    ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0)
                                });
                                if !frame_is_nonflat(&current.frame) {
                                    return Err(flat_error);
                                }
                                let input = spatial_fold_input(
                                    spatial.as_ref(),
                                    &current.frame,
                                    &self.faces,
                                    line,
                                    keep_side_point,
                                    target_layers.as_deref(),
                                    direction,
                                );
                                let mut result = ori3_layers::fold_from_plane_3d(
                                    &doc,
                                    &self.faces,
                                    up_to,
                                    &input,
                                );
                                if let Some(mut step) = result.step.take() {
                                    step.id = next_step_id(&doc, &step_creases);
                                    step.alignment = alignment;
                                    let lines = added_crease_lines(
                                        &doc.cp,
                                        &result.cp,
                                        &result.added_edges,
                                    );
                                    record_step_creases(&mut step_creases, step.id, lines);
                                    doc.cp = result.cp;
                                    doc.sequence.insert(up_to, step);
                                }
                                warnings.append(&mut insert_warnings);
                                warnings.append(&mut result.warnings);
                            }
                        }
                    }
                }
            }
            SeqOp::PreviewFoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                pose_before,
            } => {
                check_insert_point(&doc, up_to)?;
                if target_layers.is_some() && target_pleat_count.is_some() {
                    return Err("折るひだの枚数と個別の紙を同時には指定できません".to_string());
                }
                if target_pleat_count.is_some() && spatial.is_some() {
                    return Err(
                        "折るひだの枚数と3D上のつかみ位置を同時には指定できません".to_string()
                    );
                }
                let target_layers = if let Some(count) = target_pleat_count {
                    Some(fold_target_faces_at(
                        &doc,
                        &self.faces,
                        up_to,
                        line,
                        keep_side_point,
                        pose_before.as_ref(),
                        count,
                    )?)
                } else {
                    target_layers
                };
                if pose_before.is_some() && spatial.is_some() {
                    return Err(
                        "折った形の再現と3D上のつかみ位置を、同時には指定できません".to_owned()
                    );
                }
                if let Some(pose_input) = pose_before {
                    let pose = ori3_layers::replay::canonical_flat_pose_at(
                        &doc,
                        &self.faces,
                        up_to,
                        &pose_input,
                    )?;
                    fold_through_proposal = ori3_layers::propose_fold_through(
                        &doc.cp,
                        &self.faces,
                        &pose.state,
                        &ori3_layers::FoldThroughInput {
                            line,
                            keep_side_point,
                            target_layers,
                            direction,
                        },
                    )?;
                    warnings = pose.warnings;
                } else {
                    let current = spatial
                        .as_ref()
                        .map(|_| ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0));
                    if let Some(current) = current
                        .as_ref()
                        .filter(|current| frame_is_nonflat(&current.frame))
                    {
                        let input = spatial_fold_input(
                            spatial.as_ref(),
                            &current.frame,
                            &self.faces,
                            line,
                            keep_side_point,
                            target_layers.as_deref(),
                            direction,
                        );
                        warnings =
                            ori3_layers::fold_from_plane_3d(&doc, &self.faces, up_to, &input)
                                .warnings;
                    } else {
                        match ori3_layers::flat_state_at(&doc, &self.faces, up_to) {
                            Ok((state, state_warnings)) => {
                                fold_through_proposal = ori3_layers::propose_fold_through(
                                    &doc.cp,
                                    &self.faces,
                                    &state,
                                    &ori3_layers::FoldThroughInput {
                                        line,
                                        keep_side_point,
                                        target_layers,
                                        direction,
                                    },
                                )?;
                                warnings = state_warnings;
                            }
                            Err(flat_error) => {
                                let current = current.unwrap_or_else(|| {
                                    ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0)
                                });
                                if !frame_is_nonflat(&current.frame) {
                                    return Err(flat_error);
                                }
                                let input = spatial_fold_input(
                                    spatial.as_ref(),
                                    &current.frame,
                                    &self.faces,
                                    line,
                                    keep_side_point,
                                    target_layers.as_deref(),
                                    direction,
                                );
                                warnings = ori3_layers::fold_from_plane_3d(
                                    &doc,
                                    &self.faces,
                                    up_to,
                                    &input,
                                )
                                .warnings;
                            }
                        }
                    }
                }
            }
            SeqOp::CreaseOnlyTop { .. }
            | SeqOp::PreviewFoldTargets { .. }
            | SeqOp::PreviewFoldTargetsOnMaterial { .. } => {
                return Err("折る操作を読み取れませんでした".to_owned());
            }
            SeqOp::FlatMotion { up_to, parts, kind } => {
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                let (state, state_warnings) = ori3_layers::flat_state_at(&doc, &self.faces, up_to)?;
                let mut cp = doc.cp.clone();
                let result = ori3_layers::flat_motion(
                    &mut cp,
                    &self.faces,
                    &state,
                    &ori3_layers::FlatMotionInput {
                        parts: parts.into_iter().map(to_layer_motion_part).collect(),
                        kind,
                    },
                )?;
                let mut step = result.step;
                step.id = next_step_id(&doc, &step_creases);
                let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                record_step_creases(&mut step_creases, step.id, lines);
                doc.cp = cp;
                doc.sequence.insert(up_to, step);
                warnings = state_warnings;
                warnings.append(&mut insert_warnings);
                warnings.extend(result.warnings);
            }
            SeqOp::Technique {
                up_to,
                kind,
                flap,
                line,
                reference_point,
                open_to_back,
                polygon,
                center,
            } => {
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                let technique = match kind {
                    TechniqueKind::Pleat => ori3_layers::pleat,
                    TechniqueKind::InsideReverse => ori3_layers::inside_reverse,
                    TechniqueKind::OutsideReverse => ori3_layers::outside_reverse,
                    TechniqueKind::Squash => ori3_layers::squash,
                    TechniqueKind::Petal => ori3_layers::petal,
                    TechniqueKind::OpenSink => ori3_layers::open_sink,
                    TechniqueKind::Swivel => ori3_layers::swivel,
                    TechniqueKind::Twist => ori3_layers::twist,
                    _ => {
                        return Err(
                            "この折り方はまだ選べません。手動の折り操作で代替してください"
                                .to_owned(),
                        );
                    }
                };
                let (state, state_warnings) = ori3_layers::flat_state_at(&doc, &self.faces, up_to)?;
                let mut cp = doc.cp.clone();
                let result = technique(
                    &mut cp,
                    &self.faces,
                    &state,
                    &ori3_layers::TechniqueInput {
                        flap,
                        line,
                        reference_point,
                        open_to_back,
                        polygon,
                        center,
                    },
                )?;
                let mut step = result.step;
                step.id = next_step_id(&doc, &step_creases);
                let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                record_step_creases(&mut step_creases, step.id, lines);
                doc.cp = cp;
                doc.sequence.insert(up_to, step);
                warnings = state_warnings;
                warnings.append(&mut insert_warnings);
                warnings.extend(result.warnings);
            }
        }
        filter_penetration_warnings(&mut warnings, doc.display.penetration_prevention_enabled);
        let mut view = self.commit(doc, step_creases, warnings);
        view.fold_through_proposal = fold_through_proposal;
        Ok(view)
    }

    pub fn pose_solve(&mut self, request: PoseSolveRequest) -> Result<PoseOutcome, String> {
        let PoseSolveRequest {
            hard,
            preferred,
            soft,
            warm_seed,
            up_to,
            t,
            mode,
        } = request;
        let mode = mode.unwrap_or_default();
        let doc = self.doc.clone();
        let faces = self.faces.clone();
        let stored_warm = self.pose_angles.clone();
        let overlap_enabled = doc.display.overlap_prevention_enabled;
        let penetration_enabled = doc.display.penetration_prevention_enabled;
        let soft = display_soft_settings(&doc, up_to, t, soft);
        let cp = &doc.cp;
        let saved_order = ori3_layers::saved_layer_order_at(&doc, &faces, up_to, t);
        let preferred = preferred.unwrap_or_default();
        let requested_targets: Vec<Driver> = preferred.iter().chain(&hard).cloned().collect();
        let explicit_warm: Option<HashMap<EdgeId, f64>> = match mode {
            PoseSolveMode::Follow => warm_seed.map(|seed| {
                seed.into_iter()
                    .map(|driver| (driver.hinge, driver.target_angle_deg))
                    .collect()
            }),
            PoseSolveMode::Canonical => None,
        };
        if mode == PoseSolveMode::Follow
            && explicit_warm
                .as_ref()
                .is_some_and(|seed| seed.values().any(|angle| !angle.is_finite()))
        {
            return Err("追従計算の出発角に有限でない値があります".to_owned());
        }
        let warm = match mode {
            PoseSolveMode::Follow => explicit_warm.as_ref().or(stored_warm.as_ref()),
            PoseSolveMode::Canonical => None,
        };
        let driver_hinges: Vec<EdgeId> = hard
            .iter()
            .chain(&preferred)
            .map(|driver| driver.hinge)
            .collect();
        let targets: Option<HashMap<EdgeId, f64>> = (!preferred.is_empty()).then(|| {
            preferred
                .iter()
                .map(|driver| (driver.hinge, driver.target_angle_deg))
                .collect()
        });
        let contact = pose_motion_contact_options(overlap_enabled, penetration_enabled);
        let document_seed = (mode == PoseSolveMode::Canonical)
            .then(|| canonical_document_seed(&doc, &faces, up_to, t));
        let motion = match mode {
            PoseSolveMode::Follow => ori3_rigid::solve_motion_with_contact_options(
                cp,
                &faces,
                &hard,
                targets.as_ref(),
                warm,
                contact,
            ),
            PoseSolveMode::Canonical => {
                ori3_rigid::motion::solve_canonical_motion_with_contact_options(
                    cp,
                    &faces,
                    &hard,
                    targets.as_ref(),
                    document_seed.as_ref(),
                    contact,
                )
            }
        };
        let mut contact_detected = motion.contact_detected;
        let surface_order_authoritative =
            usable_pose_surface_order(motion.surface_order_authoritative, &motion.result);
        let mut result = fallback_nonfinite_pose(cp, &faces, warm, motion.result);
        let mut fallback_order: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
        fallback_order.sort_unstable();
        let overlap_order =
            pose_overlap_order(&result.frame, &fallback_order, surface_order_authoritative);
        if overlap_order.authoritative {
            ori3_soft::prevent_overlap_with_order_authority(
                cp,
                &faces,
                &mut result.frame,
                ori3_soft::OverlapOrderInput {
                    start: &overlap_order.order,
                    end: &overlap_order.order,
                    progress: 0.5,
                    authoritative: true,
                },
                &ori3_soft::OverlapSettings {
                    enabled: overlap_enabled,
                    ..Default::default()
                },
            );
        }
        stamp_saved_layer_order(&mut result.frame, saved_order.as_deref());
        let intersections = if penetration_enabled {
            ori3_rigid::self_intersection_pairs(&result.frame)
        } else {
            Vec::new()
        };
        contact_detected |= penetration_enabled && !intersections.is_empty();
        let suspect_hinges = ori3_rigid::suspect_hinges_for_intersections(
            cp,
            &faces,
            &intersections,
            &driver_hinges,
        );
        let _ = add_penetration_warning_for_intersections(
            cp,
            &faces,
            &mut result.frame,
            false,
            &intersections,
        );
        let mesh = soft_mesh(
            cp,
            &faces,
            &result.frame,
            overlap_order.authoritative,
            soft.as_ref(),
        );
        let paper_intersects = pose_flat_fold_notice_intersects(
            cp,
            &requested_targets,
            contact_detected,
            !intersections.is_empty(),
        );
        let flat_fold_violations =
            flat_fold_notice_violations(cp, &requested_targets, &result.angles, paper_intersects);
        if result.closure_rms.is_finite() && result.angles.values().all(|angle| angle.is_finite()) {
            self.pose_angles = Some(result.angles.clone());
        }
        Ok(PoseOutcome {
            result,
            soft: mesh,
            suspect_hinges,
            self_intersection_pairs: intersections,
            contact_detected,
            flat_fold_violations,
        })
    }

    pub fn fold_all_preview(
        &mut self,
        percent: f64,
        warm_seed: Option<Vec<Driver>>,
    ) -> Result<FoldAllPreviewOutcome, String> {
        fold_all_preview_outcome(&self.doc, &self.faces, percent, warm_seed)
    }

    pub fn recovery_check(&mut self) -> Result<Option<RecoveryChoices>, String> {
        self.staged_recovery_choices.take().ok_or_else(|| {
            "復旧候補の一覧が準備されていません。もう一度確認してください。".to_owned()
        })
    }

    pub fn recovery_restore(
        &mut self,
        accept: bool,
        candidate_id: u64,
    ) -> Result<Option<DocumentView>, String> {
        validate_candidate_id(candidate_id)?;
        if !accept {
            if self
                .pending_recovery_source
                .as_ref()
                .is_some_and(|pending| pending.candidate_id == candidate_id)
            {
                self.pending_recovery_source = None;
            }
            return Ok(None);
        }

        let pending = self
            .pending_recovery_source
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                "復旧する作品の内容が準備されていません。もう一度候補を選んでください。".to_owned()
            })?;
        if pending.candidate_id != candidate_id {
            return Err(
                "選んだ復旧候補と準備された作品の内容が一致しません。もう一度候補を選んでください。"
                    .to_owned(),
            );
        }

        let saved = parse_saved_document(&pending.source)?;
        let view = build_document_view(&saved.document, &saved.step_creases, Vec::new());
        self.doc = saved.document;
        self.step_creases = saved.step_creases;
        self.faces.clone_from(&view.faces);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = true;
        self.path = pending.document_path;
        self.pose_angles = None;
        self.pending_open = None;
        self.pending_save = None;
        self.staged_recovery_choices = None;
        self.pending_recovery_source = None;
        Ok(Some(self.finish_document_view(view)))
    }

    pub fn proposal_generate(
        &self,
        job_id: ProposalJobId,
        skeleton: Skeleton,
        paper: Paper,
        seed: u64,
        with_fold_plan: bool,
    ) -> Result<ProposalJobResult, String> {
        let candidates =
            generate_proposal_candidates_sequential(&skeleton, &paper, seed, with_fold_plan)?;
        Ok(ProposalJobResult { job_id, candidates })
    }

    /// WASMの製品経路ではjob専用Workerのregistryが進捗を所有する。
    /// direct同期呼出しが戻った時点ではdesktopと同じくterminal jobは回収済みである。
    #[must_use]
    pub fn proposal_progress(&self, _job_id: ProposalJobId) -> Option<ProposalProgressSnapshot> {
        None
    }

    pub fn proposal_control(
        &self,
        operation: ProposalControl,
    ) -> Result<ProposalProgressSnapshot, String> {
        match operation {
            ProposalControl::Cancel { job_id } => {
                Err(format!("提案jobが見つかりません: {}", job_id.as_str()))
            }
        }
    }

    pub fn proposal_apply(
        &mut self,
        cp: CreasePattern,
        steps: Vec<FoldStep>,
    ) -> Result<DocumentView, String> {
        if extract_faces(&cp).is_empty() {
            return Err("この展開図では紙の面が作れませんでした".to_owned());
        }
        let mut seen = HashSet::new();
        for step in &steps {
            if !seen.insert(step.id) {
                return Err("同じ折り手順が二重に入っています".to_owned());
            }
        }
        let mut doc = self.doc.clone();
        doc.cp = cp;
        doc.sequence = steps;
        let view = self.commit(doc, Vec::new(), Vec::new());
        self.pose_angles = None;
        Ok(self.finish_document_view(view))
    }

    pub fn document_export(
        &self,
        kind: ExportKind,
        _path: String,
        options: ExportOptions,
    ) -> Result<Vec<FoldIssue>, String> {
        Ok(build_document_export(&self.doc, kind, options)?.fold_issues)
    }

    /// `{ "command": ..., "args": { ... } }`を型検証し、18個の受け口へ振り分ける。
    pub fn invoke_json(&mut self, request_json: &str) -> Result<String, String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.invoke_json_inner(request_json)
        }))
        .unwrap_or_else(|payload| {
            let message = if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_owned()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "詳細不明".to_owned()
            };
            Err(format!("内部エラーが発生しました: {message}"))
        })
    }

    fn invoke_json_inner(&mut self, request_json: &str) -> Result<String, String> {
        let (command, args) = parse_request(request_json)?;
        let Some(args) = args else {
            return Err(format!(
                "コマンド「{command}」にはobjectのargsフィールドが必要です。"
            ));
        };
        match command.as_str() {
            "__web_document_open_source" => {
                let args: WebDocumentOpenSourceArgs = decode_args(&command, args)?;
                self.stage_document_open(args.path, args.source);
                encode_response(&command, ())
            }
            "__web_document_save_prepare" => {
                let args: DocumentSaveArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    self.prepare_document_save(args.path.into_option())?,
                )
            }
            "__web_document_save_abort" => {
                let _: NoArgs = decode_args(&command, args)?;
                self.abort_document_save();
                encode_response(&command, ())
            }
            "__web_document_export_prepare" => {
                let args: WebDocumentExportPrepareArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    build_document_export(&self.doc, args.kind, args.options)?,
                )
            }
            "__web_recovery_set_choices" => {
                let args: WebRecoverySetChoicesArgs = decode_args(&command, args)?;
                self.stage_recovery_choices(args.choices.into_option())?;
                encode_response(&command, ())
            }
            "__web_recovery_restore_source" => {
                let args: WebRecoveryRestoreSourceArgs = decode_args(&command, args)?;
                self.stage_recovery_source(
                    args.candidate_id,
                    args.document_path.into_option(),
                    args.source,
                )?;
                encode_response(&command, ())
            }
            "__web_recovery_snapshot" => {
                let _: NoArgs = decode_args(&command, args)?;
                encode_response(&command, self.web_recovery_snapshot())
            }
            "__web_proposal_prepare" => {
                let args: WebProposalPrepareArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    prepare_web_proposal(&args.skeleton, &args.paper, args.seed)?,
                )
            }
            "__web_proposal_generate_candidate" => {
                let args: WebProposalGenerateCandidateArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    generate_web_proposal_candidate(
                        &args.skeleton,
                        &args.packing,
                        args.paper_w,
                        args.paper_h,
                    ),
                )
            }
            "__web_proposal_verify_candidate" => {
                let args: WebProposalVerifyCandidateArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    verify_web_proposal_candidate(
                        &args.skeleton,
                        &args.paper,
                        &args.packing,
                        args.candidate,
                    )?,
                )
            }
            "document_new" => {
                let args: DocumentNewArgs = decode_args(&command, args)?;
                encode_response(&command, self.document_new(args.paper)?)
            }
            "document_open" => {
                let args: DocumentOpenArgs = decode_args(&command, args)?;
                encode_response(&command, self.document_open(args.path)?)
            }
            "document_save" => {
                let args: DocumentSaveArgs = decode_args(&command, args)?;
                encode_response(&command, self.document_save(args.path.into_option())?)
            }
            "edit_apply" => {
                let args: EditApplyArgs = decode_args(&command, args)?;
                encode_response(&command, self.edit_apply(args.op)?)
            }
            "edit_apply_batch" => {
                let args: EditApplyBatchArgs = decode_args(&command, args)?;
                encode_response(&command, self.edit_apply_batch(args.ops)?)
            }
            "edit_undo" => {
                let _: NoArgs = decode_args(&command, args)?;
                encode_response(&command, self.edit_undo()?)
            }
            "edit_redo" => {
                let _: NoArgs = decode_args(&command, args)?;
                encode_response(&command, self.edit_redo()?)
            }
            "sequence_apply" => {
                let args: SequenceApplyArgs = decode_args(&command, args)?;
                encode_response(&command, self.sequence_apply(args.op)?)
            }
            "sequence_replay" => {
                let args: SequenceReplayArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    self.sequence_replay(args.up_to, args.t, args.soft.into_option())?,
                )
            }
            "pose_solve" => {
                let args: PoseSolveArgs = decode_args(&command, args)?;
                encode_response(&command, self.pose_solve(args.request)?)
            }
            "fold_all_preview" => {
                let args: FoldAllPreviewArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    self.fold_all_preview(args.percent, args.warm_seed.into_option())?,
                )
            }
            "recovery_check" => {
                let _: NoArgs = decode_args(&command, args)?;
                encode_response(&command, self.recovery_check()?)
            }
            "recovery_restore" => {
                let args: RecoveryRestoreArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    self.recovery_restore(args.accept, args.candidate_id)?,
                )
            }
            "proposal_generate" => {
                let args: ProposalGenerateArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    self.proposal_generate(
                        args.job_id,
                        args.skeleton,
                        args.paper,
                        args.seed,
                        args.with_fold_plan,
                    )?,
                )
            }
            "proposal_progress" => {
                let args: ProposalProgressArgs = decode_args(&command, args)?;
                encode_response(&command, self.proposal_progress(args.job_id))
            }
            "proposal_control" => {
                let args: ProposalControlArgs = decode_args(&command, args)?;
                encode_response(&command, self.proposal_control(args.operation)?)
            }
            "proposal_apply" => {
                let args: ProposalApplyArgs = decode_args(&command, args)?;
                encode_response(&command, self.proposal_apply(args.cp, args.steps)?)
            }
            "document_export" => {
                let args: DocumentExportArgs = decode_args(&command, args)?;
                encode_response(
                    &command,
                    self.document_export(args.kind, args.path, args.options)?,
                )
            }
            _ => Err(format!("不明なバックエンドコマンドです: {command}")),
        }
    }
}

struct ProposalNeverCancelled;

impl SearchCancellation for ProposalNeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn prepare_web_proposal(
    skeleton: &Skeleton,
    paper: &Paper,
    seed: u64,
) -> Result<WebProposalPreparation, String> {
    skeleton.validate()?;
    let long = paper.width_mm.max(paper.height_mm);
    if !(long > 0.0 && long.is_finite()) {
        return Err("紙のサイズは正の値にしてください".to_owned());
    }
    let (paper_w, paper_h) = (paper.width_mm / long, paper.height_mm / long);
    Ok(WebProposalPreparation {
        paper_w,
        paper_h,
        packings: pack(skeleton, paper_w, paper_h, seed, PROPOSAL_PACK_STARTS),
    })
}

fn generate_web_proposal_candidate(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> WebProposalCandidateGeneration {
    match generate(skeleton, packing, paper_w, paper_h) {
        Ok(generated) => WebProposalCandidateGeneration {
            candidate: Some(ProposalCandidate {
                cp: generated.cp,
                scale: packing.scale,
                violations: generated.violations,
                warnings: generated.warnings,
                sites: generated.sites,
                fold_plan: None,
            }),
            error: None,
        },
        Err(error) => WebProposalCandidateGeneration {
            candidate: None,
            error: Some(error),
        },
    }
}

fn verify_web_proposal_candidate(
    skeleton: &Skeleton,
    paper: &Paper,
    packing: &Packing,
    mut candidate: ProposalCandidate,
) -> Result<ProposalCandidate, String> {
    if candidate.fold_plan.is_some() {
        return Err("折り方を確認する前の提案候補を指定してください".to_owned());
    }
    candidate.fold_plan = proposal_plan_folds(
        skeleton,
        packing,
        &candidate.cp,
        &candidate.sites,
        paper,
        PROPOSAL_PLAN_BUDGET,
        &ProposalNeverCancelled,
    )
    .map_err(proposal_search_abort_message)?;
    Ok(candidate)
}

fn generate_proposal_candidates_sequential(
    skeleton: &Skeleton,
    paper: &Paper,
    seed: u64,
    with_fold_plan: bool,
) -> Result<Vec<ProposalCandidate>, String> {
    let prepared = prepare_web_proposal(skeleton, paper, seed)?;
    let mut candidates = Vec::new();
    let mut last_error = None;
    for packing in &prepared.packings {
        let generated =
            generate_web_proposal_candidate(skeleton, packing, prepared.paper_w, prepared.paper_h);
        match generated.candidate {
            Some(candidate) => {
                candidates.push(if with_fold_plan {
                    verify_web_proposal_candidate(skeleton, paper, packing, candidate)?
                } else {
                    candidate
                });
            }
            None => last_error = generated.error,
        }
    }
    if candidates.is_empty() {
        return Err(last_error.unwrap_or_else(|| {
            "この骨格を紙の上に配置できませんでした(角を減らすか短くしてみてください)".to_owned()
        }));
    }
    Ok(candidates)
}

fn proposal_search_abort_message(abort: SearchAbort) -> String {
    match abort {
        SearchAbort::WatchdogExpired => {
            "提案の探索が見張り時間を超えたため中断しました(途中の候補は返していません)".to_owned()
        }
        SearchAbort::Cancelled => {
            "提案の計算を取り消しました(途中の候補は返していません)".to_owned()
        }
    }
}

fn proposal_plan_folds(
    skeleton: &Skeleton,
    packing: &Packing,
    cp: &CreasePattern,
    sites: &[LeafSite],
    paper: &Paper,
    budget: ProposalPlanBudget,
    cancellation: &dyn SearchCancellation,
) -> Result<Option<ProposalFoldPlan>, SearchAbort> {
    let long = paper.width_mm.max(paper.height_mm);
    let (paper_w, paper_h) = (paper.width_mm / long, paper.height_mm / long);
    let mut document = Document::new(paper.clone());
    document.cp = cp.clone();
    let Ok(session) = FoldSession::new(&document) else {
        return Ok(None);
    };
    let goal = FoldGoal {
        target: FinishTarget::from_skeleton(skeleton),
        body: body_on_paper(skeleton, packing, paper_w, paper_h),
        sites: sites
            .iter()
            .map(|site| TipSite {
                leaf_id: site.circle.leaf_id,
                material: site.vertex.map_or(site.circle.center, |vertex| vertex.pos),
            })
            .collect(),
        layer_target: None,
    };
    let control = SearchControl::new(budget.watchdog, cancellation);
    let outcome = search_to_completion_with_control(
        &session,
        &goal,
        GapWeights::DEFAULT,
        budget.deterministic,
        CompletionTolerance::DEFAULT,
        &control,
    )?;
    if cancellation.is_cancelled() {
        return Err(SearchAbort::Cancelled);
    }
    let order: Vec<usize> = outcome.steps.iter().map(|step| step.mv.id).collect();
    let verified = verify_search_completion(
        &session,
        &outcome,
        &goal,
        GapWeights::DEFAULT,
        PoseScan::DEFAULT,
        CompletionTolerance::DEFAULT,
    );
    if cancellation.is_cancelled() {
        return Err(SearchAbort::Cancelled);
    }
    let report = verified.report();
    let mut walk = session.clone();
    for step in &report.steps {
        if cancellation.is_cancelled() {
            return Err(SearchAbort::Cancelled);
        }
        let Some(Ok(movement)) = walk.check_move(step.id, PROPOSAL_PLAN_REBUILD_SCAN) else {
            break;
        };
        if walk.apply(&movement).is_err() {
            break;
        }
    }
    if cancellation.is_cancelled() {
        return Err(SearchAbort::Cancelled);
    }
    let folded = walk.document();
    if folded.sequence.is_empty() {
        return Ok(None);
    }
    let details = ProposalFoldPlanDetails {
        steps: folded.sequence.clone(),
        cp: folded.cp.clone(),
        planned: order.len(),
        checked: folded.sequence.len(),
    };
    Ok(Some(ProposalFoldPlan::from_verified(verified, details)))
}

// 型注釈そのものが契約検査である。関数の削除、引数の増減、型や戻り値の変更で
// コンパイルエラーになる。
type FoldAllPreviewFn =
    fn(&mut Ori3AppCore, f64, Option<Vec<Driver>>) -> Result<FoldAllPreviewOutcome, String>;
type ProposalGenerateFn = fn(
    &Ori3AppCore,
    ProposalJobId,
    Skeleton,
    Paper,
    u64,
    bool,
) -> Result<ProposalJobResult, String>;
type DocumentExportFn =
    fn(&Ori3AppCore, ExportKind, String, ExportOptions) -> Result<Vec<FoldIssue>, String>;

const _: fn(&mut Ori3AppCore, Paper) -> Result<DocumentView, String> = Ori3AppCore::document_new;
const _: fn(&mut Ori3AppCore, String) -> Result<DocumentView, String> = Ori3AppCore::document_open;
const _: fn(&mut Ori3AppCore, Option<String>) -> Result<(), String> = Ori3AppCore::document_save;
const _: fn(&mut Ori3AppCore, EditOp) -> Result<DocumentView, String> = Ori3AppCore::edit_apply;
const _: fn(&mut Ori3AppCore, Vec<EditOp>) -> Result<DocumentView, String> =
    Ori3AppCore::edit_apply_batch;
const _: fn(&mut Ori3AppCore) -> Result<DocumentView, String> = Ori3AppCore::edit_undo;
const _: fn(&mut Ori3AppCore) -> Result<DocumentView, String> = Ori3AppCore::edit_redo;
const _: fn(&mut Ori3AppCore, Value) -> Result<DocumentView, String> = Ori3AppCore::sequence_apply;
const _: fn(&mut Ori3AppCore, usize, f64, Option<SoftSettings>) -> Result<ReplayOutcome, String> =
    Ori3AppCore::sequence_replay;
const _: fn(&mut Ori3AppCore, PoseSolveRequest) -> Result<PoseOutcome, String> =
    Ori3AppCore::pose_solve;
const _: FoldAllPreviewFn = Ori3AppCore::fold_all_preview;
const _: fn(&mut Ori3AppCore) -> Result<Option<RecoveryChoices>, String> =
    Ori3AppCore::recovery_check;
const _: fn(&mut Ori3AppCore, bool, u64) -> Result<Option<DocumentView>, String> =
    Ori3AppCore::recovery_restore;
const _: ProposalGenerateFn = Ori3AppCore::proposal_generate;
const _: fn(&Ori3AppCore, ProposalJobId) -> Option<ProposalProgressSnapshot> =
    Ori3AppCore::proposal_progress;
const _: fn(&Ori3AppCore, ProposalControl) -> Result<ProposalProgressSnapshot, String> =
    Ori3AppCore::proposal_control;
const _: fn(&mut Ori3AppCore, CreasePattern, Vec<FoldStep>) -> Result<DocumentView, String> =
    Ori3AppCore::proposal_apply;
const _: DocumentExportFn = Ori3AppCore::document_export;

#[derive(Deserialize)]
struct SpatialEnvelope {
    #[serde(default)]
    spatial: Option<SpatialFoldSpec>,
}

fn parse_sequence_operation(value: Value) -> Result<(SeqOp, Option<SpatialFoldSpec>), String> {
    let spatial = serde_json::from_value::<SpatialEnvelope>(value.clone())
        .map_err(|_| "折る位置を読み取れませんでした".to_owned())?
        .spatial;
    let operation = serde_json::from_value::<SeqOp>(value)
        .map_err(|_| "折る操作を読み取れませんでした".to_owned())?;
    Ok((operation, spatial))
}

fn record_finish_soft(operation: &mut SeqOp, display: &DisplaySettings) {
    let SeqOp::PushStep { step } = operation else {
        return;
    };
    if step.kind == TechniqueKind::Pose && step.finish_soft.is_none() {
        step.finish_soft = Some(FinishSoftSettings::from(display));
    }
}

fn recorded_soft_settings(
    document: &Document,
    up_to: usize,
    t: f64,
    live: Option<SoftSettings>,
) -> Option<SoftSettings> {
    let Some(recorded) = document.finish_soft_at(up_to, t) else {
        return live;
    };
    let mut settings = live.unwrap_or_default();
    settings.enabled = recorded.enabled;
    settings.stiffness = recorded.stiffness;
    settings.pressure = recorded.pressure;
    Some(settings)
}

fn display_soft_settings(
    document: &Document,
    up_to: usize,
    t: f64,
    live: Option<SoftSettings>,
) -> Option<SoftSettings> {
    let up_to = up_to.min(document.sequence.len());
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if document.sequence.is_empty() || (up_to == document.sequence.len() && t >= 1.0) {
        return live;
    }
    recorded_soft_settings(document, up_to, t, live)
}

fn soft_mesh(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    surface_order_authoritative: bool,
    soft: Option<&SoftSettings>,
) -> Option<SoftMesh> {
    let settings = soft?;
    if !settings.enabled || !surface_order_authoritative {
        return None;
    }
    frame_surface_rank_order(frame)?;
    let mut display_frame = frame.clone();
    for face in &mut display_frame.faces {
        face.layer = face.surface_rank;
    }
    Some(ori3_soft::relax(cp, faces, &display_frame, settings))
}

fn stamp_saved_layer_order(frame: &mut Frame3D, order: Option<&[FaceId]>) -> bool {
    let Some(order) = order else {
        return false;
    };
    if order.len() != frame.faces.len() {
        return false;
    }
    let frame_ids = frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<HashSet<_>>();
    if frame_ids.len() != frame.faces.len() {
        return false;
    }
    let ranks = order
        .iter()
        .enumerate()
        .map(|(rank, &face)| Some((face, u32::try_from(rank).ok()?)))
        .collect::<Option<HashMap<_, _>>>();
    let Some(ranks) = ranks else {
        return false;
    };
    if ranks.len() != order.len()
        || ranks.len() != frame_ids.len()
        || !frame_ids.iter().all(|face| ranks.contains_key(face))
    {
        return false;
    }
    for face in &mut frame.faces {
        face.layer = ranks[&face.face];
    }
    true
}

#[derive(Debug, PartialEq, Eq)]
struct PoseOverlapOrder {
    order: Vec<FaceId>,
    authoritative: bool,
}

fn pose_overlap_order(
    frame: &Frame3D,
    fallback_order: &[FaceId],
    surface_order_authoritative: bool,
) -> PoseOverlapOrder {
    if surface_order_authoritative && let Some(canonical) = frame_surface_rank_order(frame) {
        return PoseOverlapOrder {
            order: canonical,
            authoritative: true,
        };
    }
    PoseOverlapOrder {
        order: fallback_order.to_vec(),
        authoritative: false,
    }
}

fn pose_result_is_finite(result: &ori3_rigid::SolveResult) -> bool {
    result.closure_rms.is_finite()
        && result.angles.values().all(|angle| angle.is_finite())
        && result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        })
}

fn usable_pose_surface_order(
    reported_authoritative: bool,
    result: &ori3_rigid::SolveResult,
) -> bool {
    reported_authoritative && pose_result_is_finite(result)
}

fn fallback_nonfinite_pose(
    cp: &CreasePattern,
    faces: &[Face],
    warm: Option<&HashMap<EdgeId, f64>>,
    failed: ori3_rigid::SolveResult,
) -> ori3_rigid::SolveResult {
    if pose_result_is_finite(&failed) {
        return failed;
    }
    let Some(warm) = warm else { return failed };
    let mut previous = ori3_rigid::solve(cp, faces, &[], Some(warm));
    if previous.frame.faces.iter().any(|face| {
        face.polygon
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite())
    }) {
        return failed;
    }
    previous.converged = false;
    previous.best_effort = true;
    previous.iterations = previous.iterations.saturating_add(failed.iterations);
    for warning in failed.frame.warnings {
        if !previous.frame.warnings.contains(&warning) {
            previous.frame.warnings.push(warning);
        }
    }
    if !previous
        .frame
        .warnings
        .iter()
        .any(|warning| warning.contains("収束していません"))
    {
        previous
            .frame
            .warnings
            .push("追従計算が収束していません".to_owned());
    }
    previous
}

fn pose_motion_contact_options(
    overlap_enabled: bool,
    penetration_enabled: bool,
) -> ori3_rigid::MotionContactOptions {
    ori3_rigid::MotionContactOptions {
        detect: penetration_enabled,
        prevent: overlap_enabled,
    }
}

fn canonical_document_seed(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    t: f64,
) -> HashMap<EdgeId, f64> {
    let replay_angles = (!doc.sequence.is_empty())
        .then(|| ori3_layers::replay_with_faces(doc, faces, up_to, t).hinge_angles);
    let mut hinges: Vec<EdgeId> = doc
        .cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| edge.id)
        .collect();
    hinges.sort_unstable();
    hinges
        .into_iter()
        .map(|hinge| {
            let angle = replay_angles
                .as_ref()
                .and_then(|angles| angles.get(&hinge))
                .copied()
                .unwrap_or(0.0);
            (hinge, angle)
        })
        .collect()
}

fn fold_all_preview_outcome(
    doc: &Document,
    faces: &[Face],
    percent: f64,
    warm_seed: Option<Vec<Driver>>,
) -> Result<FoldAllPreviewOutcome, String> {
    let warm = warm_seed.map(|seed| {
        seed.into_iter()
            .map(|driver| (driver.hinge, driver.target_angle_deg))
            .collect::<HashMap<_, _>>()
    });
    let ori3_rigid::FoldAllPreviewResult {
        requested_percent,
        requested_angles,
        motion,
    } = ori3_rigid::solve_fold_all_preview_with_contact_detection(
        &doc.cp,
        faces,
        percent,
        warm.as_ref(),
        doc.display.penetration_prevention_enabled,
    )
    .map_err(|error| error.to_string())?;
    let mut result = motion.result;
    let mut next_warm_seed: Vec<Driver> = result
        .angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    next_warm_seed.sort_unstable_by_key(|driver| driver.hinge);
    let intersections = if doc.display.penetration_prevention_enabled {
        ori3_rigid::self_intersection_pairs(&result.frame)
    } else {
        Vec::new()
    };
    let contact_detected = doc.display.penetration_prevention_enabled
        && (motion.contact_detected || !intersections.is_empty());
    let requested_hinges: Vec<EdgeId> =
        requested_angles.iter().map(|driver| driver.hinge).collect();
    let suspect_hinges = ori3_rigid::suspect_hinges_for_intersections(
        &doc.cp,
        faces,
        &intersections,
        &requested_hinges,
    );
    let _ = add_penetration_warning_for_intersections(
        &doc.cp,
        faces,
        &mut result.frame,
        false,
        &intersections,
    );
    let paper_intersects = pose_flat_fold_notice_intersects(
        &doc.cp,
        &requested_angles,
        contact_detected,
        !intersections.is_empty(),
    );
    let mut flat_fold_violations =
        flat_fold_notice_violations(&doc.cp, &requested_angles, &result.angles, paper_intersects);
    if (requested_percent - 100.0).abs() <= f64::EPSILON {
        flat_fold_violations.extend(local_violations(&doc.cp));
        flat_fold_violations.sort_unstable();
        flat_fold_violations.dedup();
    }
    if !flat_fold_violations.is_empty()
        && !result
            .frame
            .warnings
            .iter()
            .any(|warning| warning == FOLD_ALL_FLAT_FOLD_WARNING)
    {
        result
            .frame
            .warnings
            .push(FOLD_ALL_FLAT_FOLD_WARNING.to_owned());
    }
    Ok(FoldAllPreviewOutcome {
        result,
        requested_percent,
        requested_angles,
        next_warm_seed,
        suspect_hinges,
        self_intersection_pairs: intersections,
        contact_detected,
        flat_fold_violations,
        layer_order: FoldAllLayerOrder::UnavailableWithoutSequence,
    })
}

/// IPC用のserde型を、汎用層演算の内部型へ移す。
fn to_layer_motion_part(part: ori3_model::MotionPart) -> ori3_layers::MotionPart {
    ori3_layers::MotionPart {
        layers: part.layers,
        region: part
            .region
            .into_iter()
            .map(|half_plane| ori3_layers::HalfPlane {
                line: half_plane.line,
                inside_point: half_plane.inside_point,
            })
            .collect(),
        transform: match part.transform {
            ori3_model::MotionTransform::Stay => ori3_layers::MotionTransform::Stay,
            ori3_model::MotionTransform::Reflect(lines) => {
                ori3_layers::MotionTransform::Reflect(lines)
            }
        },
        turn: match part.turn {
            ori3_model::LayerTurn::Keep => ori3_layers::LayerTurn::Keep,
            ori3_model::LayerTurn::Outside(direction) => ori3_layers::LayerTurn::Outside(direction),
            ori3_model::LayerTurn::Inside(direction) => ori3_layers::LayerTurn::Inside(direction),
            ori3_model::LayerTurn::Beside { anchor, direction } => {
                ori3_layers::LayerTurn::Beside { anchor, direction }
            }
        },
        reverse_layers: part.reverse_layers,
    }
}

const NONFLAT_EPS: f64 = 1e-6;

fn frame_is_nonflat(frame: &Frame3D) -> bool {
    frame.faces.iter().any(|face| {
        face.polygon
            .iter()
            .any(|point| point[2].abs() > NONFLAT_EPS)
    })
}

fn spatial_fold_input(
    spatial: Option<&SpatialFoldSpec>,
    frame: &Frame3D,
    faces: &[Face],
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
    target_layers: Option<&[FaceId]>,
    direction: ori3_model::FoldDirection,
) -> ori3_layers::SpatialFoldInput {
    if let Some(spatial) = spatial {
        return ori3_layers::SpatialFoldInput {
            plane: ori3_layers::FoldPlane3D {
                origin: [
                    (spatial.from[0] + spatial.to[0]) * 0.5,
                    (spatial.from[1] + spatial.to[1]) * 0.5,
                    (spatial.from[2] + spatial.to[2]) * 0.5,
                ],
                normal: [
                    spatial.to[0] - spatial.from[0],
                    spatial.to[1] - spatial.from[1],
                    spatial.to[2] - spatial.from[2],
                ],
            },
            grab_point: spatial.from,
            grab_face: spatial.grab_face,
            direction,
        };
    }

    let dx = line[1][0] - line[0][0];
    let dy = line[1][1] - line[0][1];
    let normal = [-dy, dx, 0.0];
    let length = dx.hypot(dy);
    let keep_signed = normal[0] * (keep_side_point[0] - line[0][0])
        + normal[1] * (keep_side_point[1] - line[0][1]);
    let keep_sign = if keep_signed < 0.0 { -1.0 } else { 1.0 };
    let unit = if length > 0.0 {
        [normal[0] / length, normal[1] / length, 0.0]
    } else {
        [0.0, 0.0, 0.0]
    };
    let grab_point = [
        line[0][0] - keep_sign * unit[0] * 0.25,
        line[0][1] - keep_sign * unit[1] * 0.25,
        -keep_sign * unit[2] * 0.25,
    ];
    let requested = target_layers
        .into_iter()
        .flatten()
        .copied()
        .find(|id| faces.iter().any(|face| face.id == *id));
    let geometric = frame.faces.iter().find_map(|face| {
        face.polygon
            .iter()
            .any(|point| {
                -keep_sign
                    * (normal[0] * (point[0] - line[0][0]) + normal[1] * (point[1] - line[0][1]))
                    > ori3_model::EPS
            })
            .then_some(face.face)
    });
    ori3_layers::SpatialFoldInput {
        plane: ori3_layers::FoldPlane3D {
            origin: [line[0][0], line[0][1], 0.0],
            normal,
        },
        grab_point,
        grab_face: requested.or(geometric).unwrap_or(0),
        direction,
    }
}

fn check_insert_point(doc: &Document, up_to: usize) -> Result<Vec<String>, String> {
    let len = doc.sequence.len();
    if up_to > len {
        return Err(format!("挿入位置 {up_to} が手順の数を超えています"));
    }
    if up_to == len {
        return Ok(Vec::new());
    }
    Ok(vec![format!(
        "手順{}の前に折りを挟みました。後ろの手順{}個は折り直した形の上で再生し直しています(合わなくなった手順は飛ばして知らせます)",
        up_to + 1,
        len - up_to
    )])
}

fn next_step_id(doc: &Document, step_creases: &[StepCreases]) -> StepId {
    doc.sequence
        .iter()
        .map(|step| step.id)
        .chain(step_creases.iter().map(|creases| creases.step))
        .max()
        .map_or(0, |maximum| maximum.saturating_add(1))
}

fn added_crease_lines(
    before: &CreasePattern,
    after: &CreasePattern,
    added: &[EdgeId],
) -> Vec<[[f64; 2]; 2]> {
    let existing: HashSet<EdgeId> = before.edges.iter().map(|edge| edge.id).collect();
    let positions: HashMap<VertexId, [f64; 2]> = after
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect();
    added
        .iter()
        .filter(|id| !existing.contains(id))
        .filter_map(|id| after.edges.iter().find(|edge| edge.id == *id))
        .filter_map(|edge| Some([*positions.get(&edge.v0)?, *positions.get(&edge.v1)?]))
        .collect()
}

fn record_frontend_step(list: &mut Vec<StepCreases>, step: &FoldStep) {
    if step.kind == TechniqueKind::Pose {
        record_step_creases(list, step.id, Vec::new());
    }
}

fn record_step_creases(list: &mut Vec<StepCreases>, step: StepId, lines: Vec<[[f64; 2]; 2]>) {
    list.retain(|creases| creases.step != step);
    list.push(StepCreases { step, lines });
}

/// 編集操作1つを履歴へ触れない候補Documentへ反映する。
fn edit_document(doc: &mut Document, op: EditOp, warnings: &mut Vec<String>) -> Result<(), String> {
    match op {
        EditOp::AddSegment { a, b, kind } => {
            ori3_cp::insert_segment(&mut doc.cp, a, b, kind);
        }
        EditOp::RemoveEdges { ids } => {
            let removable: Vec<_> = ids
                .iter()
                .copied()
                .filter(|id| !is_border(&doc.cp, *id))
                .collect();
            if removable.len() != ids.len() {
                warnings.push("輪郭線は削除できません".to_owned());
            }
            ori3_cp::remove_edges(&mut doc.cp, &removable);
        }
        EditOp::SetEdgeKind { ids, kind } => {
            let mut warned_from_border = false;
            let mut warned_to_border = false;
            for id in ids {
                let Some(edge) = doc.cp.edges.iter_mut().find(|edge| edge.id == id) else {
                    warnings.push(format!("辺ID {id} が存在しません"));
                    continue;
                };
                if edge.kind == EdgeKind::Border {
                    if !warned_from_border {
                        warnings.push("輪郭線の種類は変更できません".to_owned());
                        warned_from_border = true;
                    }
                } else if kind == EdgeKind::Border {
                    if !warned_to_border {
                        warnings.push("輪郭線へ変更することはできません".to_owned());
                        warned_to_border = true;
                    }
                } else {
                    edge.kind = kind;
                }
            }
        }
        EditOp::MoveVertex { id, to } => {
            if doc.cp.vertices.iter().any(|vertex| vertex.id == id) {
                ori3_cp::move_vertex(&mut doc.cp, id, to);
            } else {
                warnings.push(format!("頂点ID {id} が存在しません"));
            }
        }
        EditOp::SetPaper { paper } => {
            validate_paper_dimensions(&paper)?;
            if doc
                .cp
                .edges
                .iter()
                .any(|edge| edge.kind != EdgeKind::Border)
            {
                return Err("折り線がある状態では紙サイズを変更できません".to_owned());
            }
            let fresh = Document::new(paper);
            doc.paper = fresh.paper;
            doc.cp = fresh.cp;
        }
        EditOp::ReplaceCreasePattern { cp } => {
            doc.cp = cp;
            doc.sequence.clear();
        }
        EditOp::SetDisplay { mut display } => {
            let divisions = display.grid_divisions;
            if !(MIN_GRID_DIVISIONS..=MAX_GRID_DIVISIONS).contains(&divisions) {
                display.grid_divisions = divisions.clamp(MIN_GRID_DIVISIONS, MAX_GRID_DIVISIONS);
                warnings.push(format!(
                    "方眼の数は{MIN_GRID_DIVISIONS}〜{MAX_GRID_DIVISIONS}の範囲で指定してください({divisions}は{}に丸めました)",
                    display.grid_divisions
                ));
            }
            doc.display = display;
        }
    }
    Ok(())
}

struct FoldTargetLookup {
    info: FoldTargetInfo,
    analysis: Option<ori3_layers::FoldTargetAnalysis>,
}

fn unavailable_fold_target_info() -> FoldTargetInfo {
    FoldTargetInfo {
        status: FoldTargetStatus::Unavailable,
        available_count: None,
        reason: Some("この折り線で同時に折れるひだを確認できません。".to_string()),
        top_action: None,
    }
}

fn fold_target_info_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
    pose_before: Option<&ori3_model::FoldPoseInput>,
) -> FoldTargetLookup {
    let Ok((analysis, _warnings)) =
        ori3_layers::fold_target_analysis_at(doc, faces, up_to, line, keep_side_point, pose_before)
    else {
        return FoldTargetLookup {
            info: unavailable_fold_target_info(),
            analysis: None,
        };
    };

    let info = fold_target_info_from_analysis(&analysis);
    let keep_analysis = info.status != FoldTargetStatus::Unavailable;
    FoldTargetLookup {
        info,
        analysis: keep_analysis.then_some(analysis),
    }
}

fn fold_target_info_from_analysis(analysis: &ori3_layers::FoldTargetAnalysis) -> FoldTargetInfo {
    let sections = &analysis.pleats.sections;
    let Some(available_count) = analysis.pleats.scalar_count else {
        return FoldTargetInfo {
            status: FoldTargetStatus::Varies,
            available_count: None,
            reason: Some("折り線の場所によって、同時に折れるひだの枚数が異なります。".to_string()),
            top_action: None,
        };
    };
    if sections.is_empty() {
        return unavailable_fold_target_info();
    }

    let all_crease_only = available_count == 0
        && sections.iter().all(|section| {
            section.pairs_top_to_bottom.is_empty()
                && section.count_limit.is_none()
                && matches!(
                    section.top_action,
                    Some(ori3_layers::TopAction::CreaseOnlyTop { .. })
                )
        });
    if all_crease_only {
        return FoldTargetInfo {
            status: FoldTargetStatus::CreaseOnlyTop,
            available_count: Some(0),
            reason: Some("いちばん上の紙が最後まで折り重なっていないため、今回はひだをまとめて折りません。いちばん上の紙に折り目だけを付け、下の紙と3Dの形は動かしません。".to_string()),
            top_action: Some(FoldTargetTopAction::CreaseOnlyTop),
        };
    }
    if available_count == 0 {
        return unavailable_fold_target_info();
    }

    let section_is_limited = |section: &ori3_layers::PleatSectionAnalysis| {
        section.top_action.is_none()
            && section.pairs_top_to_bottom.len() == available_count
            && section.count_limit
                == Some(ori3_layers::PleatCountLimit::IncompleteBoundaryAfter {
                    count: available_count,
                })
    };
    let section_is_ready = |section: &ori3_layers::PleatSectionAnalysis| {
        section.top_action.is_none()
            && section.count_limit.is_none()
            && section.pairs_top_to_bottom.len() == available_count
    };
    let all_ready_or_limited = sections
        .iter()
        .all(|section| section_is_ready(section) || section_is_limited(section));
    let any_limited = sections.iter().any(section_is_limited);
    let targets_are_safe = (1..=available_count)
        .all(|count| ori3_layers::target_faces_for_pleat_count(analysis, count).is_ok());
    if !targets_are_safe || !all_ready_or_limited {
        return unavailable_fold_target_info();
    }

    let (status, reason) = if any_limited {
        (
            FoldTargetStatus::Limited,
            Some(format!(
                "上から{available_count}枚まで選べます。{available_count}枚目の下は、まだ最後まで折り重なっていません。"
            )),
        )
    } else {
        (FoldTargetStatus::Ready, None)
    };
    FoldTargetInfo {
        status,
        available_count: Some(available_count),
        reason,
        top_action: None,
    }
}

fn fold_target_faces_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
    pose_before: Option<&ori3_model::FoldPoseInput>,
    count: usize,
) -> Result<Vec<FaceId>, String> {
    let lookup = fold_target_info_at(doc, faces, up_to, line, keep_side_point, pose_before);
    let reason = lookup
        .info
        .reason
        .clone()
        .unwrap_or_else(|| "この枚数のひだを折れません。".to_string());
    let analysis = lookup.analysis.ok_or(reason.clone())?;
    if !matches!(
        lookup.info.status,
        FoldTargetStatus::Ready | FoldTargetStatus::Limited
    ) {
        return Err(reason);
    }
    ori3_layers::target_faces_for_pleat_count(&analysis, count).map_err(|_| reason)
}

fn is_border(cp: &CreasePattern, id: EdgeId) -> bool {
    cp.edges
        .iter()
        .any(|edge| edge.id == id && edge.kind == EdgeKind::Border)
}

/// desktopのbuild_viewと同じ、新規作品用のhost-neutralな導出。
fn build_initial_document_view(doc: &Document) -> DocumentView {
    build_document_view(doc, &[], Vec::new())
}

/// desktopのbuild_viewと同じ、候補Documentからのhost-neutralな導出。
fn build_document_view(
    doc: &Document,
    step_creases: &[StepCreases],
    mut warnings: Vec<String>,
) -> DocumentView {
    warnings.extend(validate(&doc.cp));
    DocumentView {
        doc: doc.clone(),
        step_creases: retain_existing_steps(doc, step_creases),
        fold_issues: Vec::new(),
        faces: extract_faces(&doc.cp),
        warnings,
        violations: local_violations(&doc.cp),
        flat_fold_violations: Vec::new(),
        frame: None,
        skipped: Vec::new(),
        suspect_hinges: Vec::new(),
        self_intersection_pairs: Vec::new(),
        contact_detected: false,
        sequence_targets: Vec::new(),
        angles: HashMap::new(),
        relaxations: Vec::new(),
        closure_rms: None,
        best_effort: false,
        converged: true,
        fold_through_proposal: None,
        fold_target_info: None,
    }
}

/// MoveStep候補の完全な返却viewを、store確定より前に導出する。
fn build_move_step_view(doc: &Document, step_creases: &[StepCreases]) -> DocumentView {
    let mut view = build_document_view(doc, step_creases, Vec::new());
    attach_replay(&mut view);
    view
}

fn retain_existing_steps(doc: &Document, step_creases: &[StepCreases]) -> Vec<StepCreases> {
    step_creases
        .iter()
        .filter(|creases| doc.sequence.iter().any(|step| step.id == creases.step))
        .cloned()
        .collect()
}

const FLAT_TARGET_EPS_DEG: f64 = 1e-6;

fn pose_flat_fold_notice_intersects(
    cp: &CreasePattern,
    targets: &[Driver],
    contact_detected: bool,
    final_intersects: bool,
) -> bool {
    if final_intersects {
        return true;
    }
    if !contact_detected {
        return false;
    }

    let latest_targets: HashMap<EdgeId, f64> = targets
        .iter()
        .map(|target| (target.hinge, target.target_angle_deg))
        .collect();
    let mut crease_edges = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .peekable();
    let requests_every_crease_flat = crease_edges.peek().is_some()
        && crease_edges.all(|edge| {
            latest_targets.get(&edge.id).is_some_and(|target| {
                target.is_finite() && (target.abs() - 180.0).abs() <= FLAT_TARGET_EPS_DEG
            })
        });
    !requests_every_crease_flat
}

/// ±180°を指定した折り目の局所平坦条件を、desktopと同じ条件で通知へ変換する。
fn flat_fold_notice_violations(
    cp: &CreasePattern,
    targets: &[Driver],
    angles: &HashMap<EdgeId, f64>,
    paper_intersects: bool,
) -> Vec<VertexId> {
    let mut flat_targets: HashMap<EdgeId, f64> = targets
        .iter()
        .map(|target| (target.hinge, target.target_angle_deg))
        .collect();
    flat_targets.retain(|_, target| {
        target.is_finite() && (target.abs() - 180.0).abs() <= FLAT_TARGET_EPS_DEG
    });
    let edge_kinds: HashMap<EdgeId, EdgeKind> =
        cp.edges.iter().map(|edge| (edge.id, edge.kind)).collect();
    flat_targets.retain(|hinge, _| {
        edge_kinds
            .get(hinge)
            .is_some_and(|kind| *kind != EdgeKind::Border)
    });
    if flat_targets.is_empty() {
        return Vec::new();
    }

    let mut requested_cp = cp.clone();
    for edge in &mut requested_cp.edges {
        if edge.kind == EdgeKind::Border {
            continue;
        }
        edge.kind = match flat_targets.get(&edge.id) {
            Some(target) if target.is_sign_positive() => EdgeKind::Mountain,
            Some(_) => EdgeKind::Valley,
            None => EdgeKind::Aux,
        };
    }
    let violations = local_violations(&requested_cp);
    if violations.is_empty() {
        return violations;
    }
    let missed = flat_targets.iter().any(|(hinge, target)| {
        angles.get(hinge).is_none_or(|actual| {
            !actual.is_finite() || (actual - target).abs() > FLAT_TARGET_EPS_DEG
        })
    });
    if missed || paper_intersects {
        violations
    } else {
        Vec::new()
    }
}

fn replay_flat_fold_notice_violations(
    cp: &CreasePattern,
    targets: &[Driver],
    angles: &HashMap<EdgeId, f64>,
    intersections: &[(FaceId, FaceId)],
) -> Vec<VertexId> {
    flat_fold_notice_violations(cp, targets, angles, !intersections.is_empty())
}

fn attach_replay_contact_diagnostic(view: &mut DocumentView, intersections: &[(FaceId, FaceId)]) {
    view.self_intersection_pairs = intersections.to_vec();
    view.contact_detected = !intersections.is_empty();
}

fn frame_surface_rank_order(frame: &Frame3D) -> Option<Vec<FaceId>> {
    let mut face_ids = HashSet::with_capacity(frame.faces.len());
    let mut ranked = Vec::with_capacity(frame.faces.len());
    for face in &frame.faces {
        if !face_ids.insert(face.face) {
            return None;
        }
        ranked.push((face.surface_rank, face.face));
    }
    ranked.sort_unstable();
    if ranked
        .iter()
        .enumerate()
        .any(|(index, (rank, _))| u32::try_from(index).ok() != Some(*rank))
    {
        return None;
    }
    Some(ranked.into_iter().map(|(_, face)| face).collect())
}

fn replay_surface_rank_order(replayed: &ori3_layers::ReplayResult) -> Option<Vec<FaceId>> {
    replayed.surface_order_provenance.as_ref()?;
    frame_surface_rank_order(&replayed.frame)
}

fn prevent_replay_overlap_if_authoritative(
    cp: &CreasePattern,
    faces: &[Face],
    replayed: &mut ori3_layers::ReplayResult,
    settings: &ori3_soft::OverlapSettings,
) -> Option<ori3_soft::OverlapReport> {
    let order = replay_surface_rank_order(replayed)?;
    let progress = replayed.layer_transition.progress;
    Some(ori3_soft::prevent_overlap_with_order_authority(
        cp,
        faces,
        &mut replayed.frame,
        ori3_soft::OverlapOrderInput {
            start: &order,
            end: &order,
            progress,
            authoritative: true,
        },
        settings,
    ))
}

fn add_penetration_warning_for_intersections(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    check_layer_order: bool,
    intersections: &[(FaceId, FaceId)],
) -> Vec<&'static str> {
    let mut added = Vec::new();
    if !intersections.is_empty()
        && !frame
            .warnings
            .iter()
            .any(|warning| warning == ori3_rigid::PENETRATION_WARNING)
    {
        frame
            .warnings
            .push(ori3_rigid::PENETRATION_WARNING.to_owned());
        added.push(ori3_rigid::PENETRATION_WARNING);
    }
    if check_layer_order && let Some(warning) = add_layer_order_warning(cp, faces, frame) {
        added.push(warning);
    }
    added
}

fn filter_penetration_warnings(warnings: &mut Vec<String>, detect: bool) {
    if detect {
        return;
    }
    warnings.retain(|warning| {
        warning != ori3_rigid::PENETRATION_WARNING
            && warning != ori3_layers::FOLD_PENETRATION_WARNING
    });
}

fn add_layer_order_warning(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
) -> Option<&'static str> {
    correct_layer_order(cp, faces, frame);
    add_layer_order_warning_only(cp, faces, frame)
}

fn correct_layer_order(cp: &CreasePattern, faces: &[Face], frame: &mut Frame3D) {
    if ori3_rigid::layer_order_conflicts(cp, faces, frame)
        && let Some(order) = ori3_rigid::derive_layer_order(cp, faces, frame)
    {
        let rank: HashMap<FaceId, u32> = order
            .iter()
            .enumerate()
            .map(|(index, &id)| (id, u32::try_from(index).unwrap_or(u32::MAX)))
            .collect();
        for face in &mut frame.faces {
            if let Some(&layer) = rank.get(&face.face) {
                face.layer = layer;
            }
        }
    }
}

fn add_layer_order_warning_only(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
) -> Option<&'static str> {
    if !ori3_rigid::layer_order_conflicts(cp, faces, frame)
        || frame
            .warnings
            .iter()
            .any(|warning| warning == ori3_layers::FOLD_PENETRATION_WARNING)
    {
        return None;
    }
    frame
        .warnings
        .push(ori3_layers::FOLD_PENETRATION_WARNING.to_owned());
    Some(ori3_layers::FOLD_PENETRATION_WARNING)
}

fn apply_layer_order_display_settings(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    canonical_order: Option<&[FaceId]>,
    prevent: bool,
    detect: bool,
) -> Option<&'static str> {
    if prevent {
        correct_layer_order(cp, faces, frame);
        if let Some(order) = canonical_order {
            ori3_rigid::stamp_surface_order(frame, order)
                .expect("検証済みcanonical順は同じframeへ刻印できる");
        }
    }
    detect
        .then(|| add_layer_order_warning_only(cp, faces, frame))
        .flatten()
}

/// desktop store::attach_replayと同じ、最新手順までの自動再生後処理。
fn attach_replay(view: &mut DocumentView) {
    attach_replay_contact_diagnostic(view, &[]);
    if view.doc.sequence.is_empty() {
        return;
    }
    let up_to = view.doc.sequence.len();
    let mut replayed = ori3_layers::replay_with_faces(&view.doc, &view.faces, up_to, 1.0);
    let saved_order = ori3_layers::saved_layer_order_at(&view.doc, &view.faces, up_to, 1.0);
    let canonical_order = replay_surface_rank_order(&replayed);
    view.sequence_targets = replayed.sequence_targets.clone();
    view.angles = replayed.hinge_angles.clone();
    view.relaxations = replayed.relaxations.clone();
    view.closure_rms = replayed
        .closure_rms
        .is_finite()
        .then_some(replayed.closure_rms);
    view.best_effort = replayed.best_effort;
    view.converged = replayed.converged;
    let mut penetration_warnings: Vec<&'static str> = Vec::new();
    if saved_order.is_none() {
        let warning = apply_layer_order_display_settings(
            &view.doc.cp,
            &view.faces,
            &mut replayed.frame,
            canonical_order.as_deref(),
            view.doc.display.overlap_prevention_enabled,
            view.doc.display.penetration_prevention_enabled,
        );
        if let Some(warning) = warning {
            penetration_warnings.push(warning);
        }
    }
    let intersections = if view.doc.display.penetration_prevention_enabled {
        ori3_rigid::self_intersection_pairs(&replayed.frame)
    } else {
        Vec::new()
    };
    attach_replay_contact_diagnostic(view, &intersections);
    let overlap_settings = ori3_soft::OverlapSettings {
        enabled: view.doc.display.overlap_prevention_enabled,
        ..Default::default()
    };
    let _ = prevent_replay_overlap_if_authoritative(
        &view.doc.cp,
        &view.faces,
        &mut replayed,
        &overlap_settings,
    );
    view.flat_fold_violations = replay_flat_fold_notice_violations(
        &view.doc.cp,
        &replayed.sequence_targets,
        &replayed.hinge_angles,
        &intersections,
    );
    replayed.suspect_hinges = ori3_rigid::suspect_hinges_for_intersections(
        &view.doc.cp,
        &view.faces,
        &intersections,
        &replayed.driver_hinges,
    );
    penetration_warnings.extend(add_penetration_warning_for_intersections(
        &view.doc.cp,
        &view.faces,
        &mut replayed.frame,
        false,
        &intersections,
    ));
    for warning in penetration_warnings {
        if !replayed.warnings.iter().any(|existing| existing == warning) {
            replayed.warnings.push(warning.to_owned());
        }
    }
    filter_penetration_warnings(
        &mut view.warnings,
        view.doc.display.penetration_prevention_enabled,
    );
    filter_penetration_warnings(
        &mut replayed.warnings,
        view.doc.display.penetration_prevention_enabled,
    );
    filter_penetration_warnings(
        &mut replayed.frame.warnings,
        view.doc.display.penetration_prevention_enabled,
    );
    view.warnings.extend(replayed.warnings);
    view.skipped = replayed.skipped;
    view.suspect_hinges = replayed.suspect_hinges;
    view.frame = Some(replayed.frame);
}

fn validate_paper_dimensions(paper: &Paper) -> Result<(), String> {
    if paper.width_mm > 0.0 && paper.height_mm > 0.0 {
        Ok(())
    } else {
        Err("紙のサイズは正の値で指定してください".to_owned())
    }
}

fn validate_candidate_id(candidate_id: u64) -> Result<(), String> {
    if candidate_id == 0 || candidate_id > MAX_SAFE_CANDIDATE_ID {
        return Err("復旧候補の番号が安全な整数の範囲を超えています。".to_owned());
    }
    Ok(())
}

fn validate_recovery_choices(choices: &RecoveryChoices) -> Result<(), String> {
    if choices.choices.is_empty() {
        return Err("復旧候補が無い場合は null を指定してください。".to_owned());
    }
    let expected_overflow = choices.choices.len().saturating_sub(3);
    if choices.overflow_count != expected_overflow {
        return Err("復旧候補の省略件数が一覧と一致しません。".to_owned());
    }
    let mut seen = HashSet::new();
    for choice in &choices.choices {
        validate_candidate_id(choice.candidate_id)?;
        if !seen.insert(choice.candidate_id) {
            return Err("復旧候補の番号が重複しています。".to_owned());
        }
    }
    if choices.choices.windows(2).any(|pair| {
        (pair[0].saved_at_ms, pair[0].candidate_id) < (pair[1].saved_at_ms, pair[1].candidate_id)
    }) {
        return Err("復旧候補は新しい順に並べてください。".to_owned());
    }
    Ok(())
}

fn is_fold_path(path: &str) -> bool {
    path.rsplit(|character| character == '/' || character == '\\')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fold"))
}

fn parse_saved_document(source: &str) -> Result<SavedDocument, String> {
    let value: Value = serde_json::from_str(source)
        .map_err(|error| format!("ファイルの内容を読み取れませんでした: {error}"))?;
    match value.get("schema_version").and_then(Value::as_u64) {
        None => return Err("作品ファイルの形式ではありません".to_owned()),
        Some(version) if version > u64::from(SCHEMA_VERSION) => {
            return Err(
                "このファイルは新しい版のアプリで作られています。アプリを更新してください"
                    .to_owned(),
            );
        }
        Some(version) if version < u64::from(SCHEMA_VERSION) => {
            return Err(format!(
                "このファイルの形式(版{version})には対応していません"
            ));
        }
        Some(_) => {}
    }
    serde_json::from_value(value)
        .map_err(|error| format!("ファイルの内容を読み取れませんでした: {error}"))
}

fn export_png_long_side(value: i64) -> Result<u32, String> {
    if value <= 0 {
        return Err(format!("画像の大きさは1以上にしてください(指定: {value})"));
    }
    if value > i64::from(ori3_export::MAX_LONG_SIDE_PX) {
        return Err(format!(
            "画像の大きさは{}までにしてください(指定: {value})",
            ori3_export::MAX_LONG_SIDE_PX
        ));
    }
    Ok(value as u32)
}

fn export_file(suffix: String, content_type: &str, bytes: Vec<u8>) -> DocumentExportFile {
    DocumentExportFile {
        suffix,
        content_type: content_type.to_owned(),
        content_base64: encode_base64(&bytes),
        page_number: None,
        first_cell: None,
        last_cell: None,
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded =
        String::with_capacity((bytes.len() / 3 + usize::from(bytes.len() % 3 > 0)) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let bits = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        encoded.push(char::from(TABLE[((bits >> 18) & 0x3f) as usize]));
        encoded.push(char::from(TABLE[((bits >> 12) & 0x3f) as usize]));
        encoded.push(char::from(TABLE[((bits >> 6) & 0x3f) as usize]));
        encoded.push(char::from(TABLE[(bits & 0x3f) as usize]));
    }
    match chunks.remainder() {
        [] => {}
        [first] => {
            let bits = u32::from(*first) << 16;
            encoded.push(char::from(TABLE[((bits >> 18) & 0x3f) as usize]));
            encoded.push(char::from(TABLE[((bits >> 12) & 0x3f) as usize]));
            encoded.push('=');
            encoded.push('=');
        }
        [first, second] => {
            let bits = (u32::from(*first) << 16) | (u32::from(*second) << 8);
            encoded.push(char::from(TABLE[((bits >> 18) & 0x3f) as usize]));
            encoded.push(char::from(TABLE[((bits >> 12) & 0x3f) as usize]));
            encoded.push(char::from(TABLE[((bits >> 6) & 0x3f) as usize]));
            encoded.push('=');
        }
        _ => unreachable!("chunks_exact(3)の余りは2byte以下"),
    }
    encoded
}

fn build_document_export(
    document: &Document,
    kind: ExportKind,
    options: ExportOptions,
) -> Result<DocumentExportPreparation, String> {
    let svg_options = CpSvgOptions {
        include_aux: options.include_aux,
    };
    let mut fold_issues = Vec::new();
    let files = match kind {
        ExportKind::CpSvg => vec![export_file(
            String::new(),
            "image/svg+xml",
            cp_svg(document, &svg_options).into_bytes(),
        )],
        ExportKind::CpPng => vec![export_file(
            String::new(),
            "image/png",
            cp_png(
                document,
                &svg_options,
                export_png_long_side(options.png_long_side)?,
            )?,
        )],
        ExportKind::DiagramPdf => vec![export_file(
            String::new(),
            "application/pdf",
            diagram_pdf(document)?,
        )],
        ExportKind::DiagramSvg => diagram_svg_pages(document)?
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                let mut file = export_file(
                    format!("-{:02}", index + 1),
                    "image/svg+xml",
                    page.into_bytes(),
                );
                file.page_number = Some(index + 1);
                if index > 0 {
                    file.first_cell = Some((index - 1) * 6 + 1);
                    file.last_cell = Some((index * 6).min(document.sequence.len()));
                }
                file
            })
            .collect(),
        ExportKind::FoldJson => {
            let user_error = || {
                "この作品は、ほかの折り紙ソフトで使えるファイルとして書き出せません。作品の内容を確認してください。"
                    .to_owned()
            };
            let FoldExport { file, warnings } =
                document_to_fold(document).map_err(|_| user_error())?;
            let json = write_fold_1_2(&file).map_err(|_| user_error())?;
            fold_issues = warnings;
            vec![export_file(
                String::new(),
                "application/json",
                json.into_bytes(),
            )]
        }
    };
    Ok(DocumentExportPreparation { files, fold_issues })
}

fn parse_request(request_json: &str) -> Result<(String, Option<Value>), String> {
    let request: Value = serde_json::from_str(request_json)
        .map_err(|error| format!("コマンド要求のJSONを解析できません: {error}"))?;
    let object = request
        .as_object()
        .ok_or_else(|| "コマンド要求はJSON objectにしてください。".to_owned())?;
    if let Some(name) = object
        .keys()
        .find(|name| name.as_str() != "command" && name.as_str() != "args")
    {
        return Err(format!("コマンド要求に余分なフィールドがあります: {name}"));
    }
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "コマンド要求には文字列の command フィールドが必要です。".to_owned())?
        .to_owned();
    if !BACKEND_COMMAND_NAMES.contains(&command.as_str())
        && !WEB_HOST_COMMAND_NAMES.contains(&command.as_str())
    {
        return Err(format!("不明なバックエンドコマンドです: {command}"));
    }
    let Some(args) = object.get("args") else {
        if matches!(
            command.as_str(),
            "edit_undo" | "edit_redo" | "recovery_check"
        ) {
            return Ok((command, Some(serde_json::json!({}))));
        }
        return Ok((command, None));
    };
    if args.is_null()
        && matches!(
            command.as_str(),
            "edit_undo" | "edit_redo" | "recovery_check"
        )
    {
        return Ok((command, Some(serde_json::json!({}))));
    }
    if !args.is_object() {
        return Err("コマンド要求の args フィールドはobjectにしてください。".to_owned());
    }
    ensure_finite_numbers(args)?;
    Ok((command, Some(args.clone())))
}

fn ensure_finite_numbers(value: &Value) -> Result<(), String> {
    match value {
        Value::Array(values) => values.iter().try_for_each(ensure_finite_numbers),
        Value::Object(values) => values.values().try_for_each(ensure_finite_numbers),
        Value::Number(number) => match number.as_f64() {
            Some(number) if number.is_finite() => Ok(()),
            _ => Err("コマンドの数値には有限の値を指定してください。".to_owned()),
        },
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
    }
}

fn decode_args<T: DeserializeOwned>(command: &str, args: Value) -> Result<T, String> {
    serde_json::from_value(args)
        .map_err(|error| format!("コマンド「{command}」の引数を読み取れません: {error}"))
}

fn encode_response<T: Serialize>(command: &str, response: T) -> Result<String, String> {
    serde_json::to_string(&response)
        .map_err(|error| format!("コマンド「{command}」の応答をJSONにできません: {error}"))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RequiredNullable<T> {
    Value(T),
    Null(()),
}

impl<T> RequiredNullable<T> {
    fn into_option(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Null(()) => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentNewArgs {
    paper: Paper,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentOpenArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentSaveArgs {
    path: RequiredNullable<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebDocumentOpenSourceArgs {
    path: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditApplyArgs {
    op: EditOp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditApplyBatchArgs {
    ops: Vec<EditOp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceApplyArgs {
    op: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SequenceReplayArgs {
    up_to: usize,
    t: f64,
    soft: RequiredNullable<SoftSettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PoseSolveArgs {
    request: PoseSolveRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FoldAllPreviewArgs {
    percent: f64,
    warm_seed: RequiredNullable<Vec<Driver>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryRestoreArgs {
    accept: bool,
    candidate_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProposalGenerateArgs {
    job_id: ProposalJobId,
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
    with_fold_plan: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WebProposalPrepareArgs {
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WebProposalGenerateCandidateArgs {
    skeleton: Skeleton,
    packing: Packing,
    paper_w: f64,
    paper_h: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebProposalVerifyCandidateArgs {
    skeleton: Skeleton,
    paper: Paper,
    packing: Packing,
    candidate: ProposalCandidate,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProposalProgressArgs {
    job_id: ProposalJobId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalControlArgs {
    operation: ProposalControl,
}

#[derive(Deserialize)]
struct ProposalApplyArgs {
    cp: CreasePattern,
    steps: Vec<FoldStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentExportArgs {
    kind: ExportKind,
    path: String,
    options: ExportOptions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebDocumentExportPrepareArgs {
    kind: ExportKind,
    options: ExportOptions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebRecoverySetChoicesArgs {
    choices: RequiredNullable<RecoveryChoices>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WebRecoveryRestoreSourceArgs {
    candidate_id: u64,
    document_path: RequiredNullable<String>,
    source: String,
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use ori3_model::{
        CreasePattern, DisplaySettings, Document, DriverLine, EPS, EdgeKind, EditOp, FoldStep,
        MIN_GRID_DIVISIONS, Paper, SCHEMA_VERSION, StepCreases, TechniqueKind,
    };
    use ori3_propose::skeleton::{Skeleton, SkeletonNode};
    use serde_json::{Value, json};

    use super::{
        BACKEND_COMMAND_NAMES, DocumentView, Ori3AppCore, Snapshot, build_document_export,
        build_document_view,
    };

    fn valid_args(command: &str) -> Value {
        let paper = json!({ "width_mm": 150.0, "height_mm": 150.0 });
        let cp = json!({
            "vertices": [],
            "edges": [],
            "next_vertex_id": 0,
            "next_edge_id": 0
        });
        match command {
            "document_new" => json!({ "paper": paper }),
            "document_open" => json!({ "path": "作品.ori3" }),
            "document_save" => json!({ "path": null }),
            "edit_apply" => json!({
                "op": { "type": "SetPaper", "paper": paper }
            }),
            "edit_apply_batch" => json!({ "ops": [] }),
            "edit_undo" | "edit_redo" | "recovery_check" => json!({}),
            "sequence_apply" => json!({
                "op": { "type": "RemoveStep", "id": 1 }
            }),
            "sequence_replay" => json!({ "upTo": 0, "t": 1.0, "soft": null }),
            "pose_solve" => json!({
                "request": {
                    "hard": [],
                    "preferred": null,
                    "soft": null,
                    "warmSeed": null,
                    "upTo": 0,
                    "t": 1.0,
                    "mode": "Follow"
                }
            }),
            "fold_all_preview" => json!({ "percent": 50.0, "warmSeed": null }),
            "recovery_restore" => json!({ "accept": false, "candidateId": 1 }),
            "proposal_generate" => json!({
                "jobId": "job-1",
                "skeleton": { "nodes": [] },
                "paper": paper,
                "seed": 1,
                "withFoldPlan": true
            }),
            "proposal_progress" => json!({ "jobId": "job-1" }),
            "proposal_control" => json!({
                "operation": { "type": "Cancel", "job_id": "job-1" }
            }),
            "proposal_apply" => json!({ "cp": cp, "steps": [] }),
            "document_export" => json!({
                "kind": "CpSvg",
                "path": "作品.svg",
                "options": { "include_aux": false, "png_long_side": 2048 }
            }),
            _ => unreachable!("固定長コマンド表以外を検査しない"),
        }
    }

    fn request(command: &str, args: Value) -> String {
        json!({ "command": command, "args": args }).to_string()
    }

    fn new_150x100_request() -> String {
        request(
            "document_new",
            json!({ "paper": { "width_mm": 150.0, "height_mm": 100.0 } }),
        )
    }

    fn add_diagonal_request() -> String {
        request(
            "edit_apply",
            json!({
                "op": {
                    "type": "AddSegment",
                    "a": [0.0, 0.0],
                    "b": [1.0, 2.0 / 3.0],
                    "kind": "Mountain"
                }
            }),
        )
    }

    fn remove_diagonal_batch_request() -> String {
        request(
            "edit_apply_batch",
            json!({
                "ops": [
                    { "type": "SetEdgeKind", "ids": [4], "kind": "Valley" },
                    { "type": "RemoveEdges", "ids": [4] }
                ]
            }),
        )
    }

    fn preview_fold_through_request() -> String {
        request(
            "sequence_apply",
            json!({
                "op": {
                    "type": "PreviewFoldThrough",
                    "up_to": 0,
                    "line": [[0.0, 0.0], [1.0, 2.0 / 3.0]],
                    "keep_side_point": [0.0, 2.0 / 3.0],
                    "target_layers": null,
                    "direction": "Up"
                }
            }),
        )
    }

    fn apply_fold_through_request() -> String {
        request(
            "sequence_apply",
            json!({
                "op": {
                    "type": "FoldThrough",
                    "up_to": 0,
                    "line": [[0.0, 0.0], [1.0, 2.0 / 3.0]],
                    "keep_side_point": [0.0, 2.0 / 3.0],
                    "target_layers": null,
                    "direction": "Up",
                    "accept_additional_crease": false
                }
            }),
        )
    }

    fn replay_fold_through_half_request() -> String {
        request(
            "sequence_replay",
            json!({ "upTo": 1, "t": 0.5, "soft": null }),
        )
    }

    fn pose_step_value(id: u32, note: &str) -> Value {
        json!({
            "id": id,
            "kind": "Pose",
            "drivers": [],
            "layer_order": null,
            "note": note
        })
    }

    fn pose_solve_diagonal_request() -> String {
        request(
            "pose_solve",
            json!({
                "request": {
                    "hard": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                    "preferred": null,
                    "soft": null,
                    "warmSeed": null,
                    "upTo": 0,
                    "t": 1.0,
                    "mode": "Follow"
                }
            }),
        )
    }

    fn pose_solve_diagonal_canonical_request(warm_seed: Value) -> String {
        request(
            "pose_solve",
            json!({
                "request": {
                    "hard": [],
                    "preferred": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                    "soft": null,
                    "warmSeed": warm_seed,
                    "upTo": 0,
                    "t": 1.0,
                    "mode": "Canonical"
                }
            }),
        )
    }

    fn fold_all_preview_diagonal_zero_request() -> String {
        request(
            "fold_all_preview",
            json!({ "percent": 0.0, "warmSeed": null }),
        )
    }

    fn fold_all_preview_diagonal_request(warm_seed: Value) -> String {
        request(
            "fold_all_preview",
            json!({ "percent": 50.0, "warmSeed": warm_seed }),
        )
    }

    fn diagonal_core() -> Ori3AppCore {
        let mut core = Ori3AppCore::new();
        core.invoke_json(&new_150x100_request())
            .expect("new command must succeed");
        core.invoke_json(&add_diagonal_request())
            .expect("diagonal edit must succeed");
        core
    }

    fn fold_all_three_strips(left: EdgeKind, right: EdgeKind) -> Document {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let vertex = |id, x, y| ori3_model::Vertex { id, pos: [x, y] };
        let edge = |id, v0, v1, kind| ori3_model::Edge { id, v0, v1, kind };
        document.cp = CreasePattern {
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0 / 3.0, 0.0),
                vertex(2, 2.0 / 3.0, 0.0),
                vertex(3, 1.0, 0.0),
                vertex(4, 1.0, 1.0),
                vertex(5, 2.0 / 3.0, 1.0),
                vertex(6, 1.0 / 3.0, 1.0),
                vertex(7, 0.0, 1.0),
            ],
            edges: vec![
                edge(0, 0, 1, EdgeKind::Border),
                edge(1, 1, 2, EdgeKind::Border),
                edge(2, 2, 3, EdgeKind::Border),
                edge(3, 3, 4, EdgeKind::Border),
                edge(4, 4, 5, EdgeKind::Border),
                edge(5, 5, 6, EdgeKind::Border),
                edge(6, 6, 7, EdgeKind::Border),
                edge(7, 7, 0, EdgeKind::Border),
                edge(8, 1, 6, left),
                edge(9, 2, 5, right),
            ],
            next_vertex_id: 8,
            next_edge_id: 10,
        };
        document
    }

    fn penetrating_fold_all_case() -> (Document, Vec<ori3_cp::Face>, f64) {
        static CASE: std::sync::OnceLock<(EdgeKind, EdgeKind, u8)> = std::sync::OnceLock::new();
        let &(left, right, percent) = CASE.get_or_init(|| {
            for (left, right) in [
                (EdgeKind::Mountain, EdgeKind::Mountain),
                (EdgeKind::Mountain, EdgeKind::Valley),
                (EdgeKind::Valley, EdgeKind::Mountain),
                (EdgeKind::Valley, EdgeKind::Valley),
            ] {
                let document = fold_all_three_strips(left, right);
                let faces = ori3_cp::extract_faces(&document.cp);
                for percent in (5..=95).step_by(5) {
                    let outcome = super::fold_all_preview_outcome(
                        &document,
                        &faces,
                        f64::from(percent),
                        None,
                    )
                    .expect("3短冊の一斉折り姿勢を返す");
                    if !outcome.self_intersection_pairs.is_empty() {
                        return (left, right, percent);
                    }
                }
            }
            panic!("3短冊の山谷4通り×5〜95%に貫通姿勢がなく、検査標本になっていない")
        });
        let document = fold_all_three_strips(left, right);
        let faces = ori3_cp::extract_faces(&document.cp);
        (document, faces, f64::from(percent))
    }

    fn fold_all_core_with_detection(enabled: bool) -> (Ori3AppCore, f64) {
        let (mut document, _faces, percent) = penetrating_fold_all_case();
        document.display.penetration_prevention_enabled = enabled;
        let source = serde_json::to_string(&document).expect("3短冊作品をJSONへ保存できる");
        let path = "browser-file://read/session/self-intersection-three-strips.ori3";
        let mut core = Ori3AppCore::new();
        core.invoke_json(&request(
            "__web_document_open_source",
            json!({ "path": path, "source": source }),
        ))
        .expect("ブラウザが3短冊作品の本文を渡せる");
        core.invoke_json(&request("document_open", json!({ "path": path })))
            .expect("ブラウザ経路で3短冊作品を開ける");
        (core, percent)
    }

    fn bird_base_proposal_skeleton() -> Skeleton {
        Skeleton {
            nodes: vec![
                SkeletonNode::new(0, None, 0.0),
                SkeletonNode::new(1, Some(0), 1.0),
                SkeletonNode::new(2, Some(0), 1.0),
                SkeletonNode::new(3, Some(0), 0.8),
                SkeletonNode::new(4, Some(0), 0.8),
            ],
        }
    }

    fn numbered_step(id: u32) -> FoldStep {
        FoldStep {
            id,
            kind: TechniqueKind::Simple,
            drivers: Vec::new(),
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: String::new(),
        }
    }

    fn assert_complete_replay(view: &DocumentView, core: &Ori3AppCore) {
        assert!(view.frame.is_some());
        assert!(view.skipped.is_empty());
        assert_eq!(view.sequence_targets.len(), 1);
        assert!(view.angles.contains_key(&4));
        assert!(view.angles.values().all(|angle| angle.is_finite()));
        assert!(view.closure_rms.is_some_and(f64::is_finite));
        assert!(!view.best_effort);
        assert!(view.converged);
        assert!(!view.contact_detected);
        assert!(view.suspect_hinges.is_empty());
        assert_eq!(core.pose_angles.as_ref(), Some(&view.angles));
    }

    #[test]
    fn command_table_has_exactly_eighteen_unique_names() {
        assert_eq!(BACKEND_COMMAND_NAMES.len(), 18);
        assert_eq!(
            BACKEND_COMMAND_NAMES
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            18
        );
    }

    #[test]
    fn edit_apply_batch_undo_redo_share_one_atomic_history_and_complete_json() {
        let mut core = Ori3AppCore::new();
        core.invoke_json(&new_150x100_request())
            .expect("new command must succeed");
        assert!(core.undo_stack.is_empty());

        let applied = core
            .invoke_json(&add_diagonal_request())
            .expect("apply command must succeed");
        assert_eq!(core.undo_stack.len(), 1);
        assert!(core.redo_stack.is_empty());
        let applied_value: Value =
            serde_json::from_str(&applied).expect("apply response must be JSON");
        assert_eq!(
            applied_value["doc"]["cp"]["edges"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(applied_value["doc"]["cp"]["edges"][4]["id"], 4);
        assert_eq!(applied_value["doc"]["cp"]["edges"][4]["kind"], "Mountain");
        assert_eq!(applied_value["faces"].as_array().unwrap().len(), 2);

        let batched = core
            .invoke_json(&remove_diagonal_batch_request())
            .expect("batch command must succeed");
        assert_eq!(core.undo_stack.len(), 2, "the whole batch is one undo unit");
        assert!(core.redo_stack.is_empty());
        let batched_value: Value =
            serde_json::from_str(&batched).expect("batch response must be JSON");
        assert_eq!(
            batched_value["doc"]["cp"]["edges"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(batched_value["faces"].as_array().unwrap().len(), 1);

        let undone = core
            .invoke_json(&request("edit_undo", json!({})))
            .expect("undo command must succeed");
        assert_eq!(undone, applied);
        assert_eq!(core.undo_stack.len(), 1);
        assert_eq!(core.redo_stack.len(), 1);

        let redone = core
            .invoke_json(&request("edit_redo", json!({})))
            .expect("redo command must succeed");
        assert_eq!(redone, batched);
        assert_eq!(core.undo_stack.len(), 2);
        assert!(core.redo_stack.is_empty());

        assert_eq!(
            applied,
            include_str!("../tests/fixtures/edit-apply-diagonal-150x100.json").trim_end()
        );
        assert_eq!(
            batched,
            include_str!("../tests/fixtures/edit-apply-batch-remove-diagonal-150x100.json")
                .trim_end()
        );
    }

    #[test]
    fn edit_rejections_are_atomic_and_partial_warnings_match_desktop() {
        let mut core = Ori3AppCore::new();
        core.document_new(Paper {
            width_mm: 150.0,
            height_mm: 100.0,
        })
        .expect("new document must succeed");

        let before_empty = core.clone();
        let empty_error = match core.edit_apply_batch(Vec::new()) {
            Ok(_) => panic!("empty batch must fail"),
            Err(error) => error,
        };
        assert_eq!(empty_error, "編集する内容がありません");
        assert_eq!(core, before_empty);

        let before_partial = core.clone();
        let error = match core.edit_apply_batch(vec![
            EditOp::AddSegment {
                a: [0.0, 0.0],
                b: [1.0, 2.0 / 3.0],
                kind: EdgeKind::Mountain,
            },
            EditOp::SetPaper {
                paper: Paper {
                    width_mm: 100.0,
                    height_mm: 100.0,
                },
            },
        ]) {
            Ok(_) => panic!("a failing later op must reject the whole batch"),
            Err(error) => error,
        };
        assert_eq!(error, "折り線がある状態では紙サイズを変更できません");
        assert_eq!(core, before_partial);

        let border = core
            .edit_apply(EditOp::RemoveEdges { ids: vec![0] })
            .expect("border removal is a warning");
        assert_eq!(border.warnings, vec!["輪郭線は削除できません"]);
        assert!(core.undo_stack.is_empty());

        let missing = core
            .edit_apply(EditOp::MoveVertex {
                id: 999,
                to: [0.5, 0.5],
            })
            .expect("missing vertex is a warning");
        assert_eq!(missing.warnings, vec!["頂点ID 999 が存在しません"]);
        assert!(core.undo_stack.is_empty());

        let kinds = core
            .edit_apply(EditOp::SetEdgeKind {
                ids: vec![0, 999],
                kind: EdgeKind::Valley,
            })
            .expect("mixed edge IDs use partial warning semantics");
        assert_eq!(
            kinds.warnings,
            vec!["輪郭線の種類は変更できません", "辺ID 999 が存在しません"]
        );
        assert!(core.undo_stack.is_empty());

        let mut display = core.doc.display.clone();
        display.grid_divisions = 1;
        let clamped = core
            .edit_apply(EditOp::SetDisplay { display })
            .expect("display range is clamped with a warning");
        assert_eq!(clamped.doc.display.grid_divisions, MIN_GRID_DIVISIONS);
        assert_eq!(
            clamped.warnings,
            vec!["方眼の数は2〜1024の範囲で指定してください(1は2に丸めました)"]
        );
        assert_eq!(core.undo_stack.len(), 1);
    }

    #[test]
    fn edit_history_ignores_noops_clears_redo_and_keeps_exactly_one_hundred() {
        let mut core = Ori3AppCore::new();
        let display = core.doc.display.clone();
        core.edit_apply(EditOp::SetDisplay { display })
            .expect("same display is a successful no-op");
        assert!(core.undo_stack.is_empty());

        for value in 0_u8..=100 {
            let mut display = core.doc.display.clone();
            display.front_color[0] = value;
            core.edit_apply(EditOp::SetDisplay { display })
                .expect("history seed edit must succeed");
        }
        assert_eq!(core.undo_stack.len(), 100);

        core.edit_undo().expect("undo must succeed");
        assert_eq!(core.redo_stack.len(), 1);
        let mut display = core.doc.display.clone();
        display.back_color[0] = 0;
        core.edit_apply(EditOp::SetDisplay { display })
            .expect("new branch edit must succeed");
        assert!(core.redo_stack.is_empty());
        let redo_error = match core.edit_redo() {
            Ok(_) => panic!("cleared branch cannot be redone"),
            Err(error) => error,
        };
        assert_eq!(redo_error, "これ以上やり直せません");
    }

    #[test]
    fn all_four_edit_commands_replay_nonempty_sequence_and_store_finite_angles() {
        let mut core = Ori3AppCore::new();
        core.document_new(Paper {
            width_mm: 150.0,
            height_mm: 100.0,
        })
        .expect("new document must succeed");
        core.edit_apply(EditOp::AddSegment {
            a: [0.0, 0.0],
            b: [1.0, 2.0 / 3.0],
            kind: EdgeKind::Mountain,
        })
        .expect("diagonal must be inserted");
        core.doc.sequence.push(FoldStep {
            id: 7,
            kind: TechniqueKind::Simple,
            drivers: vec![DriverLine {
                a: [0.0, 0.0],
                b: [1.0, 2.0 / 3.0],
                target_angle_deg: 180.0,
            }],
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: String::new(),
        });
        core.step_creases.push(StepCreases {
            step: 7,
            lines: Vec::new(),
        });
        core.undo_stack.clear();
        core.redo_stack.clear();
        core.dirty = false;
        core.pose_angles = None;

        let mut applied_display = core.doc.display.clone();
        applied_display.grid_divisions = 9;
        let applied = core
            .edit_apply(EditOp::SetDisplay {
                display: applied_display,
            })
            .expect("apply must replay the sequence");
        assert_complete_replay(&applied, &core);
        assert_eq!(applied.step_creases, core.step_creases);
        let applied_json =
            serde_json::to_string(&applied).expect("apply view must serialize deterministically");

        let mut first_display = core.doc.display.clone();
        first_display.grid_divisions = 10;
        let mut final_display = first_display.clone();
        final_display.grid_divisions = 11;
        let batched = core
            .edit_apply_batch(vec![
                EditOp::SetDisplay {
                    display: first_display,
                },
                EditOp::SetDisplay {
                    display: final_display,
                },
            ])
            .expect("batch must replay the sequence");
        assert_complete_replay(&batched, &core);
        assert_eq!(core.undo_stack.len(), 2);
        let batched_json =
            serde_json::to_string(&batched).expect("batch view must serialize deterministically");

        let undone = core.edit_undo().expect("undo must replay the sequence");
        assert_complete_replay(&undone, &core);
        assert_eq!(
            serde_json::to_string(&undone).expect("undo view must serialize"),
            applied_json
        );

        let redone = core.edit_redo().expect("redo must replay the sequence");
        assert_complete_replay(&redone, &core);
        assert_eq!(
            serde_json::to_string(&redone).expect("redo view must serialize"),
            batched_json
        );
    }

    #[test]
    fn json_guard_converts_edit_panic_and_keeps_precommit_state() {
        let mut core = Ori3AppCore::new();
        core.invoke_json(&new_150x100_request())
            .expect("new command must succeed");
        let mut cp = core.doc.cp.clone();
        cp.next_vertex_id = u32::MAX;
        core.invoke_json(&request(
            "edit_apply",
            json!({ "op": { "type": "ReplaceCreasePattern", "cp": cp } }),
        ))
        .expect("replace CP itself must remain a normal edit");

        let before = core.clone();
        let error = core
            .invoke_json(&request(
                "edit_apply",
                json!({
                    "op": {
                        "type": "AddSegment",
                        "a": [0.25, 0.25],
                        "b": [0.75, 0.5],
                        "kind": "Mountain"
                    }
                }),
            ))
            .expect_err("debug overflow panic must cross the JSON guard as Err");
        assert!(error.starts_with("内部エラーが発生しました: "));
        assert_eq!(core, before);

        let continued = core
            .edit_apply(EditOp::MoveVertex {
                id: 999,
                to: [0.5, 0.5],
            })
            .expect("core must remain usable after guarded panic");
        assert_eq!(continued.warnings, vec!["頂点ID 999 が存在しません"]);
    }

    #[test]
    fn all_eighteen_public_commands_have_a_rust_core_implementation() {
        let commands = BACKEND_COMMAND_NAMES
            .into_iter()
            .filter(|command| {
                !matches!(
                    *command,
                    "document_new"
                        | "edit_apply"
                        | "edit_apply_batch"
                        | "edit_undo"
                        | "edit_redo"
                        | "sequence_apply"
                        | "sequence_replay"
                        | "pose_solve"
                        | "fold_all_preview"
                        | "recovery_check"
                        | "recovery_restore"
                        | "document_open"
                        | "document_save"
                        | "document_export"
                        | "proposal_generate"
                        | "proposal_progress"
                        | "proposal_control"
                        | "proposal_apply"
                )
            })
            .collect::<Vec<_>>();
        assert!(
            commands.is_empty(),
            "公開18件がすべて実装済み: {commands:?}"
        );
    }

    #[test]
    fn bird_base_proposal_split_matches_public_result_and_preserves_candidate_order() {
        let skeleton = bird_base_proposal_skeleton();
        let paper = Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        };
        let mut direct_core = Ori3AppCore::new();
        let direct: Value = serde_json::from_str(
            &direct_core
                .invoke_json(&request(
                    "proposal_generate",
                    json!({
                        "jobId": "bird-base-direct",
                        "skeleton": skeleton,
                        "paper": paper,
                        "seed": 1,
                        "withFoldPlan": true
                    }),
                ))
                .expect("鳥の基本形候補を生成できる"),
        )
        .expect("公開結果をJSONとして読める");

        let mut split_core = Ori3AppCore::new();
        let prepared: Value = serde_json::from_str(
            &split_core
                .invoke_json(&request(
                    "__web_proposal_prepare",
                    json!({ "skeleton": skeleton, "paper": paper, "seed": 1 }),
                ))
                .expect("提案Worker向けの充填を準備できる"),
        )
        .expect("充填結果をJSONとして読める");
        let packings = prepared["packings"].as_array().expect("packingsは配列");
        assert!(!packings.is_empty());
        assert!(packings.len() <= 4);

        let mut split_candidates = Vec::new();
        for packing in packings {
            let generated: Value = serde_json::from_str(
                &split_core
                    .invoke_json(&request(
                        "__web_proposal_generate_candidate",
                        json!({
                            "skeleton": skeleton,
                            "packing": packing,
                            "paperW": prepared["paper_w"],
                            "paperH": prepared["paper_h"]
                        }),
                    ))
                    .expect("候補単位で共通生成計算を呼べる"),
            )
            .expect("候補生成結果をJSONとして読める");
            if !generated["candidate"].is_null() {
                let verified: Value = serde_json::from_str(
                    &split_core
                        .invoke_json(&request(
                            "__web_proposal_verify_candidate",
                            json!({
                                "skeleton": skeleton,
                                "paper": paper,
                                "packing": packing,
                                "candidate": generated["candidate"]
                            }),
                        ))
                        .expect("候補単位で共通の折り方探索と検証を呼べる"),
                )
                .expect("検証済み候補をJSONとして読める");
                split_candidates.push(verified);
            }
        }
        assert_eq!(direct["job_id"], "bird-base-direct");
        assert_eq!(direct["candidates"], Value::Array(split_candidates));
    }

    #[test]
    fn proposal_apply_is_one_atomic_operation_and_rejection_changes_nothing() {
        let mut core = diagonal_core();
        let before = core.clone();
        let replacement_cp = Ori3AppCore::new().doc.cp;
        let steps = vec![numbered_step(0), numbered_step(1)];
        let applied = core
            .proposal_apply(replacement_cp.clone(), steps.clone())
            .expect("提案の展開図と手順を同時に適用できる");
        assert_eq!(applied.doc.cp, replacement_cp);
        assert_eq!(applied.doc.sequence, steps);
        assert_eq!(core.undo_stack.len(), before.undo_stack.len() + 1);
        assert_eq!(core.edit_undo().expect("1回で元に戻せる").doc, before.doc);
        assert_eq!(core.doc, before.doc);

        let before_rejection = core.clone();
        let error = match core.proposal_apply(
            Ori3AppCore::new().doc.cp,
            vec![numbered_step(7), numbered_step(7)],
        ) {
            Ok(_) => panic!("二重の手順番号を受理した"),
            Err(error) => error,
        };
        assert_eq!(error, "同じ折り手順が二重に入っています");
        assert_eq!(core, before_rejection);

        let error = match core.proposal_apply(
            CreasePattern {
                vertices: Vec::new(),
                edges: Vec::new(),
                next_vertex_id: 0,
                next_edge_id: 0,
            },
            vec![numbered_step(8)],
        ) {
            Ok(_) => panic!("面を作れない展開図を受理した"),
            Err(error) => error,
        };
        assert_eq!(error, "この展開図では紙の面が作れませんでした");
        assert_eq!(core, before_rejection);
    }

    #[test]
    fn terminal_proposal_progress_is_pruned_and_unknown_cancel_matches_desktop() {
        let mut core = Ori3AppCore::new();
        assert_eq!(
            core.invoke_json(&request(
                "proposal_progress",
                json!({ "jobId": "finished-bird-base" }),
            ))
            .expect("完了済みjobの進捗はnull"),
            "null"
        );
        let error = core
            .invoke_json(&request(
                "proposal_control",
                json!({
                    "operation": { "type": "Cancel", "job_id": "finished-bird-base" }
                }),
            ))
            .expect_err("未知jobの取消しを受理しない");
        assert_eq!(error, "提案jobが見つかりません: finished-bird-base");
    }

    #[test]
    fn recovery_check_returns_exact_newest_first_choices_once_and_rejects_bad_host_data() {
        let choices = json!({
            "choices": [
                {
                    "autosave_path": "browser-recovery://4",
                    "document_path": "newest.ori3",
                    "saved_at_ms": 10,
                    "candidate_id": 4,
                    "step_count": 4
                },
                {
                    "autosave_path": "browser-recovery://3",
                    "document_path": "tie-b.ori3",
                    "saved_at_ms": 5,
                    "candidate_id": 3,
                    "step_count": 3
                },
                {
                    "autosave_path": "browser-recovery://2",
                    "document_path": "tie-a.ori3",
                    "saved_at_ms": 5,
                    "candidate_id": 2,
                    "step_count": 2
                },
                {
                    "autosave_path": "browser-recovery://1",
                    "document_path": null,
                    "saved_at_ms": null,
                    "candidate_id": 1,
                    "step_count": null
                }
            ],
            "overflow_count": 1
        });
        let mut core = Ori3AppCore::new();
        assert_eq!(
            core.invoke_json(&request(
                "__web_recovery_set_choices",
                json!({ "choices": choices })
            ))
            .unwrap(),
            "null"
        );
        assert_eq!(
            serde_json::from_str::<Value>(
                &core
                    .invoke_json(&request("recovery_check", json!({})))
                    .unwrap()
            )
            .unwrap(),
            choices
        );
        assert_eq!(
            core.invoke_json(&request("recovery_check", json!({})))
                .unwrap_err(),
            "復旧候補の一覧が準備されていません。もう一度確認してください。"
        );

        core.invoke_json(&request(
            "__web_recovery_set_choices",
            json!({ "choices": choices }),
        ))
        .unwrap();
        let before = core.clone();
        let mut wrong_overflow = choices.clone();
        wrong_overflow["overflow_count"] = json!(0);
        assert_eq!(
            core.invoke_json(&request(
                "__web_recovery_set_choices",
                json!({ "choices": wrong_overflow })
            ))
            .unwrap_err(),
            "復旧候補の省略件数が一覧と一致しません。"
        );
        assert_eq!(core, before);

        let mut missing_step_count = choices.clone();
        missing_step_count["choices"][0]
            .as_object_mut()
            .unwrap()
            .remove("step_count");
        let direct_error =
            serde_json::from_value::<super::RecoveryInfo>(missing_step_count["choices"][0].clone())
                .unwrap_err()
                .to_string();
        assert!(direct_error.contains("step_count"), "{direct_error}");
        let missing_error = core
            .invoke_json(&request(
                "__web_recovery_set_choices",
                json!({ "choices": missing_step_count }),
            ))
            .unwrap_err();
        assert!(
            missing_error.contains("引数を読み取れません"),
            "{missing_error}"
        );
        assert_eq!(core, before);

        core.invoke_json(&request(
            "__web_recovery_set_choices",
            json!({ "choices": null }),
        ))
        .unwrap();
        assert_eq!(
            core.invoke_json(&request("recovery_check", json!({})))
                .unwrap(),
            "null"
        );
    }

    #[test]
    fn recovery_restore_preserves_current_sequence_history_and_replays_the_same_view() {
        let mut source_core = diagonal_core();
        source_core.doc.sequence.push(FoldStep {
            id: 7,
            kind: TechniqueKind::Simple,
            drivers: vec![DriverLine {
                a: [0.0, 0.0],
                b: [1.0, 2.0 / 3.0],
                target_angle_deg: 180.0,
            }],
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: "current saved document".to_owned(),
        });
        source_core.step_creases.push(StepCreases {
            step: 7,
            lines: vec![[[0.0, 0.0], [1.0, 2.0 / 3.0]]],
        });
        let saved = source_core.saved_document();
        let source = serde_json::to_string_pretty(&saved).unwrap();
        assert!(source.contains("step_creases"));
        let mut expected = build_document_view(&saved.document, &saved.step_creases, Vec::new());
        super::attach_replay(&mut expected);

        let mut core = Ori3AppCore::new();
        core.invoke_json(&request(
            "__web_recovery_restore_source",
            json!({
                "candidateId": 6,
                "documentPath": "current.ori3",
                "source": source
            }),
        ))
        .unwrap();
        let response: Value = serde_json::from_str(
            &core
                .invoke_json(&request(
                    "recovery_restore",
                    json!({ "accept": true, "candidateId": 6 }),
                ))
                .unwrap(),
        )
        .unwrap();

        assert_eq!(response, serde_json::to_value(&expected).unwrap());
        assert_eq!(core.doc, saved.document);
        assert_eq!(core.step_creases, saved.step_creases);
        assert_eq!(core.pose_angles.as_ref(), Some(&expected.angles));
    }

    #[test]
    fn recovery_restore_uses_rust_saved_document_parser_and_is_atomic_on_errors() {
        let source_core = diagonal_core();
        let saved = source_core.saved_document();
        let source = serde_json::to_string_pretty(&saved).unwrap();
        assert!(
            !source.contains("step_creases"),
            "legacy SavedDocument without step_creases remains readable"
        );
        let path = "browser-file://recovery/restored.ori3";
        let mut core = Ori3AppCore::new();
        core.pose_angles = Some(HashMap::new());
        core.invoke_json(&request(
            "__web_recovery_restore_source",
            json!({
                "candidateId": 7,
                "documentPath": path,
                "source": source
            }),
        ))
        .unwrap();
        let response = core
            .invoke_json(&request(
                "recovery_restore",
                json!({ "accept": true, "candidateId": 7 }),
            ))
            .unwrap();
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            response["doc"],
            serde_json::to_value(&saved.document).unwrap()
        );
        assert_eq!(core.doc, saved.document);
        assert_eq!(core.step_creases, saved.step_creases);
        assert!(core.undo_stack.is_empty());
        assert!(core.redo_stack.is_empty());
        assert!(core.dirty);
        assert_eq!(core.path.as_deref(), Some(path));
        assert_eq!(core.pose_angles, None);
        assert_eq!(core.pending_recovery_source, None);

        let snapshot: Value = serde_json::from_str(
            &core
                .invoke_json(&request("__web_recovery_snapshot", json!({})))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(snapshot["doc"], serde_json::to_value(&core.doc).unwrap());
        assert_eq!(snapshot["step_creases"], json!([]));

        core.invoke_json(&request(
            "__web_recovery_restore_source",
            json!({
                "candidateId": 8,
                "documentPath": null,
                "source": source
            }),
        ))
        .unwrap();
        let before_mismatch = core.clone();
        assert_eq!(
            core.invoke_json(&request(
                "recovery_restore",
                json!({ "accept": true, "candidateId": 9 }),
            ))
            .unwrap_err(),
            "選んだ復旧候補と準備された作品の内容が一致しません。もう一度候補を選んでください。"
        );
        assert_eq!(core, before_mismatch);

        core.invoke_json(&request(
            "__web_recovery_restore_source",
            json!({
                "candidateId": 8,
                "documentPath": null,
                "source": "{"
            }),
        ))
        .unwrap();
        let before_bad_source = core.clone();
        assert!(
            core.invoke_json(&request(
                "recovery_restore",
                json!({ "accept": true, "candidateId": 8 }),
            ))
            .unwrap_err()
            .starts_with("ファイルの内容を読み取れませんでした:")
        );
        assert_eq!(core, before_bad_source);

        let mut rejected = before_bad_source;
        rejected.pending_recovery_source = None;
        assert_eq!(
            core.invoke_json(&request(
                "recovery_restore",
                json!({ "accept": false, "candidateId": 8 }),
            ))
            .unwrap(),
            "null"
        );
        assert_eq!(core, rejected);
    }

    #[test]
    fn web_recovery_snapshot_is_null_for_a_clean_document() {
        let mut core = Ori3AppCore::new();
        assert_eq!(
            core.invoke_json(&request("__web_recovery_snapshot", json!({})))
                .unwrap(),
            "null"
        );
    }

    #[test]
    fn web_document_save_and_ori3_open_use_the_public_commands_after_host_staging() {
        let mut source_core = diagonal_core();
        let saved = source_core.saved_document();
        let expected_content = serde_json::to_string_pretty(&saved).expect("保存JSONを作れる");
        let target = "browser-file://file-system/session/作品.ori3";

        let prepared = source_core
            .invoke_json(&request(
                "__web_document_save_prepare",
                json!({ "path": target }),
            ))
            .expect("Rust coreで保存内容を準備できる");
        let prepared: Value = serde_json::from_str(&prepared).expect("準備結果JSON");
        assert_eq!(prepared["path"], target);
        assert_eq!(prepared["content"], expected_content);
        assert!(source_core.dirty, "host書込み前は未保存のまま");
        assert_eq!(source_core.path, None);

        assert_eq!(
            source_core
                .invoke_json(&request("document_save", json!({ "path": target })))
                .expect("host書込み成功後だけ保存を確定できる"),
            "null"
        );
        assert!(!source_core.dirty);
        assert_eq!(source_core.path.as_deref(), Some(target));

        let expected_view = build_document_view(&saved.document, &saved.step_creases, Vec::new());
        let expected_view_json = serde_json::to_string(&expected_view).expect("期待view JSON");
        let mut opened = Ori3AppCore::new();
        assert_eq!(
            opened
                .invoke_json(&request(
                    "__web_document_open_source",
                    json!({ "path": target, "source": expected_content }),
                ))
                .expect("host本文をstageできる"),
            "null"
        );
        assert_eq!(
            opened
                .invoke_json(&request("document_open", json!({ "path": target })))
                .expect("公開document_openがstage済み本文を開ける"),
            expected_view_json
        );
        assert_eq!(opened.doc, saved.document);
        assert_eq!(opened.step_creases, saved.step_creases);
        assert!(opened.undo_stack.is_empty());
        assert!(opened.redo_stack.is_empty());
        assert!(!opened.dirty);
        assert_eq!(opened.path.as_deref(), Some(target));
        assert_eq!(opened.pose_angles, None);
    }

    #[test]
    fn web_document_save_failure_never_marks_the_document_saved() {
        let mut core = diagonal_core();
        let target = "browser-file://file-system/session/失敗.ori3";
        core.invoke_json(&request(
            "__web_document_save_prepare",
            json!({ "path": target }),
        ))
        .expect("保存準備は成功する");
        core.invoke_json(&request("__web_document_save_abort", json!({})))
            .expect("host書込み失敗を破棄できる");
        let before = core.clone();
        let error = core
            .invoke_json(&request("document_save", json!({ "path": target })))
            .expect_err("host書込み無しでは保存を確定しない");
        assert_eq!(
            error,
            "保存する内容が準備されていません。もう一度保存してください。"
        );
        assert_eq!(core, before);
        assert!(core.dirty);
        assert_eq!(core.path, None);
    }

    #[test]
    fn web_ori3_open_accepts_legacy_files_and_rejects_bad_files_atomically() {
        let legacy = serde_json::to_string_pretty(&Document::new(Paper {
            width_mm: 120.0,
            height_mm: 80.0,
        }))
        .expect("旧形式JSON");
        let path = "browser-file://read/session/以前の作品.ori3";
        let mut core = diagonal_core();
        core.invoke_json(&request(
            "__web_document_open_source",
            json!({ "path": path, "source": legacy }),
        ))
        .expect("旧形式本文をstageできる");
        let opened = core
            .invoke_json(&request("document_open", json!({ "path": path })))
            .expect("step_creasesを持たない既存作品を開ける");
        let opened: Value = serde_json::from_str(&opened).expect("旧形式のDocumentView JSON");
        assert!(
            opened.get("self_intersection_pairs").is_none(),
            "空の面ペアは旧形式と同じwire形を保つ"
        );
        assert!(core.step_creases.is_empty());
        assert_eq!(core.doc.paper.width_mm, 120.0);
        assert_eq!(core.doc.paper.height_mm, 80.0);

        let before = core.clone();
        for (source, expected) in [
            ("{}", "作品ファイルの形式ではありません"),
            (
                r#"{"schema_version":999}"#,
                "このファイルは新しい版のアプリで作られています。アプリを更新してください",
            ),
            (
                r#"{"schema_version":0}"#,
                "このファイルの形式(版0)には対応していません",
            ),
        ] {
            core.invoke_json(&request(
                "__web_document_open_source",
                json!({ "path": path, "source": source }),
            ))
            .expect("壊れた本文もhost stage自体はできる");
            let error = core
                .invoke_json(&request("document_open", json!({ "path": path })))
                .expect_err("不正作品は開かない");
            assert_eq!(error, expected);
            assert_eq!(core, before);
        }
    }

    #[test]
    fn web_fold_open_is_one_dirty_undoable_import_with_structured_issues() {
        const FOLD: &str =
            include_str!("../../ori3-export/tests/fixtures/fold/flat-face-orders.fold");
        let path = "browser-file://read/session/基本形.FOLD";
        let mut core = Ori3AppCore::new();
        let before = core.doc.clone();
        core.invoke_json(&request(
            "__web_document_open_source",
            json!({ "path": path, "source": FOLD }),
        ))
        .expect("FOLD本文をstageできる");
        let opened = core
            .invoke_json(&request("document_open", json!({ "path": path })))
            .expect("FOLDを共通parser/converterで開ける");
        let opened: Value = serde_json::from_str(&opened).expect("FOLD view JSON");
        assert!(opened["fold_issues"].is_array());
        assert!(core.dirty);
        assert_eq!(core.path, None);
        assert_eq!(
            core.pose_angles.as_ref().map(HashMap::len),
            opened["angles"].as_object().map(|values| values.len())
        );
        assert_eq!(core.undo_stack.len(), 1);
        core.edit_undo().expect("FOLD取込を1回で戻せる");
        assert_eq!(core.doc, before);
    }

    #[test]
    fn web_document_export_builds_all_payloads_before_host_io_with_rust_page_ranges() {
        let options = super::ExportOptions {
            include_aux: true,
            png_long_side: 256,
        };
        let mut core = diagonal_core();
        let before = core.clone();
        let svg = build_document_export(&core.doc, super::ExportKind::CpSvg, options)
            .expect("CP SVGを作れる");
        assert_eq!(svg.files.len(), 1);
        assert_eq!(svg.files[0].content_type, "image/svg+xml");
        assert!(svg.files[0].content_base64.starts_with("PD94bWwg"));
        let png = build_document_export(&core.doc, super::ExportKind::CpPng, options)
            .expect("CP PNGを作れる");
        assert!(png.files[0].content_base64.starts_with("iVBORw0KGgo"));
        let fold = build_document_export(&core.doc, super::ExportKind::FoldJson, options)
            .expect("FOLD JSONを作れる");
        assert!(fold.files[0].content_base64.starts_with("ew"));
        assert_eq!(core, before, "書き出し準備は作品stateを変えない");

        core.invoke_json(&apply_fold_through_request())
            .expect("折り図用に1手作れる");
        let before_diagram = core.clone();
        let diagram_svg = build_document_export(&core.doc, super::ExportKind::DiagramSvg, options)
            .expect("折り図SVGを作れる");
        assert_eq!(diagram_svg.files.len(), 2, "表紙と1手のページ");
        assert_eq!(diagram_svg.files[0].suffix, "-01");
        assert_eq!(diagram_svg.files[0].page_number, Some(1));
        assert_eq!(diagram_svg.files[0].first_cell, None);
        assert_eq!(diagram_svg.files[0].last_cell, None);
        assert_eq!(diagram_svg.files[1].suffix, "-02");
        assert_eq!(diagram_svg.files[1].page_number, Some(2));
        assert_eq!(diagram_svg.files[1].first_cell, Some(1));
        assert_eq!(diagram_svg.files[1].last_cell, Some(1));
        assert!(
            diagram_svg
                .files
                .iter()
                .all(|file| file.content_base64.starts_with("PD94bWwg"))
        );
        let pdf = build_document_export(&core.doc, super::ExportKind::DiagramPdf, options)
            .expect("折り図PDFを作れる");
        assert!(pdf.files[0].content_base64.starts_with("JVBERi0"));
        assert_eq!(core, before_diagram, "折り図準備も作品stateを変えない");

        let response = core
            .invoke_json(&request(
                "__web_document_export_prepare",
                json!({
                    "kind": "DiagramSvg",
                    "options": { "include_aux": true, "png_long_side": 256 }
                }),
            ))
            .expect("JSON境界でも全ページを返せる");
        let response: Value = serde_json::from_str(&response).expect("書き出し準備JSON");
        assert_eq!(response["files"].as_array().map(Vec::len), Some(2));
        assert_eq!(response["files"][1]["first_cell"], 1);
        assert_eq!(response["files"][1]["last_cell"], 1);
        assert_eq!(core, before_diagram);
    }

    #[test]
    fn web_export_base64_transport_matches_rfc_4648_vectors() {
        for (source, expected) in [
            (b"".as_slice(), ""),
            (b"f".as_slice(), "Zg=="),
            (b"fo".as_slice(), "Zm8="),
            (b"foo".as_slice(), "Zm9v"),
            (b"foob".as_slice(), "Zm9vYg=="),
            (b"fooba".as_slice(), "Zm9vYmE="),
            (b"foobar".as_slice(), "Zm9vYmFy"),
        ] {
            assert_eq!(super::encode_base64(source), expected);
        }
    }

    #[test]
    fn web_document_export_keeps_desktop_errors_and_state() {
        let core = diagonal_core();
        let before = core.clone();
        for (value, expected) in [
            (-5, "画像の大きさは1以上にしてください(指定: -5)".to_owned()),
            (
                i64::from(ori3_export::MAX_LONG_SIDE_PX) + 1,
                format!(
                    "画像の大きさは{}までにしてください(指定: {})",
                    ori3_export::MAX_LONG_SIDE_PX,
                    i64::from(ori3_export::MAX_LONG_SIDE_PX) + 1
                ),
            ),
        ] {
            let error = build_document_export(
                &core.doc,
                super::ExportKind::CpPng,
                super::ExportOptions {
                    include_aux: false,
                    png_long_side: value,
                },
            )
            .expect_err("不正な画像寸法を拒否する");
            assert_eq!(error, expected);
            assert_eq!(core, before);
        }
        for kind in [super::ExportKind::DiagramPdf, super::ExportKind::DiagramSvg] {
            assert_eq!(
                build_document_export(
                    &core.doc,
                    kind,
                    super::ExportOptions {
                        include_aux: false,
                        png_long_side: 256,
                    },
                )
                .expect_err("手順なしの折り図を拒否する"),
                "折り手順がまだありません。手順を作ってから折り図を書き出してください"
            );
            assert_eq!(core, before);
        }
    }

    #[test]
    fn document_new_replaces_all_desktop_store_state_and_builds_the_same_initial_view() {
        let mut core = Ori3AppCore::new();
        let old_snapshot = Snapshot {
            doc: core.doc.clone(),
            step_creases: core.step_creases.clone(),
        };
        core.step_creases.push(ori3_model::StepCreases {
            step: 99,
            lines: Vec::new(),
        });
        core.faces.clear();
        core.undo_stack.push(old_snapshot.clone());
        core.redo_stack.push(old_snapshot);
        core.dirty = true;
        core.path = Some("browser-file-token:old".to_owned());
        core.pose_angles = Some(HashMap::from([(0, 42.0)]));

        let view = core
            .document_new(Paper {
                width_mm: 150.0,
                height_mm: 100.0,
            })
            .expect("正の有限な紙から初期作品を作れる");

        assert_eq!(core.doc, view.doc);
        assert!(core.step_creases.is_empty());
        assert_eq!(core.faces, view.faces);
        assert!(core.undo_stack.is_empty());
        assert!(core.redo_stack.is_empty());
        assert!(!core.dirty);
        assert!(core.path.is_none());
        assert!(core.pose_angles.is_none());
        assert_eq!(view.doc.schema_version, SCHEMA_VERSION);
        assert_eq!(view.doc.paper.width_mm, 150.0);
        assert_eq!(view.doc.paper.height_mm, 100.0);
        assert_eq!(view.doc.cp.vertices.len(), 4);
        assert_eq!(view.doc.cp.edges.len(), 4);
        assert_eq!(view.doc.cp.next_vertex_id, 4);
        assert_eq!(view.doc.cp.next_edge_id, 4);
        assert_eq!(view.doc.cp.vertices[0].pos, [0.0, 0.0]);
        assert_eq!(view.doc.cp.vertices[1].pos, [1.0, 0.0]);
        assert!((view.doc.cp.vertices[2].pos[1] - 2.0 / 3.0).abs() <= EPS);
        assert!(
            view.doc
                .cp
                .edges
                .iter()
                .all(|edge| edge.kind == EdgeKind::Border)
        );
        assert!(view.doc.sequence.is_empty());
        assert_eq!(view.doc.display, DisplaySettings::default());

        assert_eq!(view.faces.len(), 1);
        assert_eq!(view.faces[0].id, 0);
        assert_eq!(view.faces[0].vertices, [0, 1, 2, 3]);
        assert_eq!(view.faces[0].edges, [0, 1, 2, 3]);
        assert!(view.step_creases.is_empty());
        assert!(view.fold_issues.is_empty());
        assert!(view.warnings.is_empty());
        assert!(view.violations.is_empty());
        assert!(view.flat_fold_violations.is_empty());
        assert!(view.frame.is_none(), "3D姿勢は初期作品へ保存しない");
        assert!(view.skipped.is_empty());
        assert!(view.suspect_hinges.is_empty());
        assert!(!view.contact_detected);
        assert!(view.sequence_targets.is_empty());
        assert!(view.angles.is_empty());
        assert!(view.relaxations.is_empty());
        assert!(view.closure_rms.is_none());
        assert!(!view.best_effort);
        assert!(
            view.converged,
            "desktopのbuild_viewは初期作品を収束済みで返す"
        );
        assert!(view.fold_through_proposal.is_none());
    }

    #[test]
    fn document_new_json_has_the_fixed_wire_shape_and_is_byte_deterministic() {
        let request = request(
            "document_new",
            json!({ "paper": { "width_mm": 150.0, "height_mm": 100.0 } }),
        );
        let mut core = Ori3AppCore::new();
        let before = core.clone();
        let first = core
            .invoke_json(&request)
            .expect("document_newの成功応答をJSONにできる");
        let after_first = core.clone();
        let second = core
            .invoke_json(&request)
            .expect("同じcoreで同じ要求を繰り返せる");
        let third = Ori3AppCore::new()
            .invoke_json(&request)
            .expect("別のcoreでも同じ要求を処理できる");
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert_eq!(first.as_bytes(), third.as_bytes());
        assert_eq!(
            first,
            include_str!("../tests/fixtures/document-new-150x100.json").trim_end(),
            "matches the complete desktop DocumentStore JSON fixture"
        );
        assert_ne!(after_first, before, "document_newはcore状態を置き換える");
        assert_eq!(core, after_first, "同じ新規作成の繰返しは同じ状態になる");

        let response: Value = serde_json::from_str(&first).expect("応答はJSON objectである");
        let object = response.as_object().expect("応答はJSON objectである");
        let actual_keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
        let expected_keys = [
            "doc",
            "step_creases",
            "fold_issues",
            "faces",
            "warnings",
            "violations",
            "flat_fold_violations",
            "frame",
            "skipped",
            "suspect_hinges",
            "contact_detected",
            "sequence_targets",
            "angles",
            "relaxations",
            "closure_rms",
            "best_effort",
            "converged",
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        assert_eq!(actual_keys, expected_keys);
        assert_eq!(response["doc"]["paper"]["width_mm"], 150.0);
        assert_eq!(response["doc"]["paper"]["height_mm"], 100.0);
        assert_eq!(
            response["doc"]["cp"]["vertices"].as_array().unwrap().len(),
            4
        );
        assert_eq!(response["doc"]["cp"]["edges"].as_array().unwrap().len(), 4);
        assert_eq!(response["faces"].as_array().unwrap().len(), 1);
        assert!(response["frame"].is_null());
        assert!(response["closure_rms"].is_null());
        assert_eq!(response["angles"], json!({}));
        assert_eq!(response["converged"], true);
        assert!(object.get("fold_through_proposal").is_none());
    }

    #[test]
    fn document_new_rejects_invalid_dimensions_in_japanese_without_changing_state() {
        let invalid = [
            (
                Paper {
                    width_mm: -1.0,
                    height_mm: 100.0,
                },
                "紙のサイズは正の値で指定してください",
            ),
            (
                Paper {
                    width_mm: 100.0,
                    height_mm: 0.0,
                },
                "紙のサイズは正の値で指定してください",
            ),
            (
                Paper {
                    width_mm: f64::NAN,
                    height_mm: 100.0,
                },
                "紙のサイズは正の値で指定してください",
            ),
            (
                Paper {
                    width_mm: 100.0,
                    height_mm: f64::NEG_INFINITY,
                },
                "紙のサイズは正の値で指定してください",
            ),
        ];
        let mut core = Ori3AppCore::new();
        for (paper, expected) in invalid {
            let before = core.clone();
            let error = match core.document_new(paper) {
                Ok(_) => panic!("不正な紙寸法を受理しない"),
                Err(error) => error,
            };
            assert_eq!(error, expected);
            assert_eq!(core, before);
        }

        let before = core.clone();
        let error = core
            .invoke_json(&request(
                "document_new",
                json!({ "paper": { "width_mm": -1.0, "height_mm": 100.0 } }),
            ))
            .expect_err("JSON境界でも負の紙寸法を受理しない");
        assert_eq!(error, "紙のサイズは正の値で指定してください");
        assert_eq!(core, before);
        assert!(
            core.invoke_json(&request(
                "document_new",
                json!({ "paper": { "width_mm": 150.0, "height_mm": 100.0 } }),
            ))
            .is_ok(),
            "不正入力の後もcoreは継続利用できる"
        );
    }

    #[test]
    fn seventeen_strict_commands_reject_extra_fields_without_changing_state() {
        let mut core = Ori3AppCore::new();
        let commands = BACKEND_COMMAND_NAMES
            .into_iter()
            .filter(|command| *command != "proposal_apply")
            .collect::<Vec<_>>();
        assert_eq!(commands.len(), 17);
        for command in commands {
            let Value::Object(mut args) = valid_args(command) else {
                unreachable!("valid_argsはobjectだけを返す")
            };
            args.insert("unexpected".to_owned(), Value::Bool(true));
            let before = core.clone();
            let error = core
                .invoke_json(&request(command, Value::Object(args)))
                .expect_err("余分な引数を受理しない");
            assert!(error.contains("引数を読み取れません"), "{command}: {error}");
            assert_eq!(core, before, "{command}の不正入力がcore状態を変更した");
        }
    }

    #[test]
    fn proposal_apply_accepts_desktop_outer_extensions_with_the_same_result() {
        let cp = Ori3AppCore::new().doc.cp;
        let steps = vec![numbered_step(0), numbered_step(1)];
        let mut exact_core = diagonal_core();
        let mut extended_core = exact_core.clone();
        let exact = exact_core
            .invoke_json(&request(
                "proposal_apply",
                json!({ "cp": cp, "steps": steps }),
            ))
            .expect("通常の提案適用を受理する");
        let extended = extended_core
            .invoke_json(&request(
                "proposal_apply",
                json!({
                    "cp": cp,
                    "steps": steps,
                    "futureDesktopField": { "keptCompatible": true }
                }),
            ))
            .expect("desktop同様に余剰の外側引数を受理する");
        assert_eq!(extended, exact);
        assert_eq!(extended_core, exact_core);
    }

    #[test]
    fn all_fifteen_nonzero_arg_commands_reject_missing_fields_without_state_change() {
        let commands = [
            "document_new",
            "document_open",
            "document_save",
            "edit_apply",
            "edit_apply_batch",
            "sequence_apply",
            "sequence_replay",
            "pose_solve",
            "fold_all_preview",
            "recovery_restore",
            "proposal_generate",
            "proposal_progress",
            "proposal_control",
            "proposal_apply",
            "document_export",
        ];
        assert_eq!(commands.len(), 15);
        let mut core = Ori3AppCore::new();
        for command in commands {
            let before = core.clone();
            let missing_args = core
                .invoke_json(&json!({ "command": command }).to_string())
                .expect_err("args objectの不足を受理しない");
            assert_eq!(
                missing_args,
                format!("コマンド「{command}」にはobjectのargsフィールドが必要です。")
            );
            assert_eq!(core, before, "{command}のargs不足がcore状態を変更した");
            let error = core
                .invoke_json(&request(command, json!({})))
                .expect_err("必須引数の不足を受理しない");
            assert!(error.contains("引数を読み取れません"), "{command}: {error}");
            assert_eq!(core, before, "{command}の必須不足がcore状態を変更した");
        }
    }

    #[test]
    fn zero_arg_commands_accept_null_and_legacy_missing_args() {
        for command in ["edit_undo", "edit_redo"] {
            for request in [
                json!({ "command": command }).to_string(),
                json!({ "command": command, "args": null }).to_string(),
            ] {
                let mut core = Ori3AppCore::new();
                let before = core.clone();
                let error = core
                    .invoke_json(&request)
                    .expect_err("empty history must be reported");
                let expected = if command == "edit_undo" {
                    "これ以上元に戻せません"
                } else {
                    "これ以上やり直せません"
                };
                assert_eq!(error, expected);
                assert_eq!(core, before);
            }
        }
        for request in [
            json!({ "command": "recovery_check" }).to_string(),
            json!({ "command": "recovery_check", "args": null }).to_string(),
        ] {
            let mut core = Ori3AppCore::new();
            let before = core.clone();
            let error = core
                .invoke_json(&request)
                .expect_err("host preparation is required");
            assert_eq!(
                error,
                "復旧候補の一覧が準備されていません。もう一度確認してください。"
            );
            assert_eq!(core, before);
        }
    }

    #[test]
    fn malformed_envelope_and_wrong_types_return_japanese_errors_without_state_change() {
        let invalid = [
            "{".to_owned(),
            "[]".to_owned(),
            json!({ "command": "document_new" }).to_string(),
            json!({ "command": "document_new", "args": null }).to_string(),
            request("document_new", json!({ "paper": "正方形" })),
            request(
                "sequence_replay",
                json!({ "upTo": -1, "t": 1.0, "soft": null }),
            ),
            request("unknown", json!({})),
        ];
        let mut core = Ori3AppCore::new();
        for request in invalid {
            let before = core.clone();
            let error = core
                .invoke_json(&request)
                .expect_err("不正要求を受理しない");
            assert!(
                error
                    .chars()
                    .any(|character| ('ぁ'..='ん').contains(&character))
            );
            assert_eq!(core, before);
        }
    }

    #[test]
    fn overflowing_float_is_rejected_before_dispatch_and_keeps_state() {
        let mut core = Ori3AppCore::new();
        let before = core.clone();
        let error = core
            .invoke_json(
                r#"{"command":"fold_all_preview","args":{"percent":1e400,"warmSeed":null}}"#,
            )
            .expect_err("有限で表せないf64を受理しない");
        assert!(error.starts_with("コマンド要求のJSONを解析できません"));
        assert_eq!(core, before);
    }

    #[test]
    fn sequence_apply_rejects_a_value_that_is_not_a_sequence_operation() {
        let mut core = Ori3AppCore::new();
        for (op, expected) in [
            (json!(123), "折る位置を読み取れませんでした"),
            (json!({}), "折る操作を読み取れませんでした"),
        ] {
            let before = core.clone();
            let error = core
                .invoke_json(&request("sequence_apply", json!({ "op": op })))
                .expect_err("invalid SeqOp must fail");
            assert_eq!(error, expected);
            assert_eq!(core, before);
        }
    }

    #[test]
    fn pose_and_fold_all_actual_frontend_flow_matches_complete_fixtures() {
        let mut core = Ori3AppCore::new();
        core.invoke_json(&new_150x100_request())
            .expect("new command must succeed");
        core.invoke_json(&add_diagonal_request())
            .expect("diagonal edit must succeed");

        let before_pose = core.clone();
        let pose = core
            .invoke_json(&pose_solve_diagonal_request())
            .expect("pose solve must succeed");
        let pose_value: Value = serde_json::from_str(&pose).expect("pose JSON");
        assert_eq!(pose_value["angles"]["4"], 90.0);
        assert_eq!(pose_value["soft"], Value::Null);
        assert_eq!(pose_value["suspect_hinges"], json!([]));
        assert_eq!(pose_value["contact_detected"], false);
        assert_eq!(pose_value["flat_fold_violations"], json!([]));
        let mut expected_pose_state = before_pose;
        expected_pose_state.pose_angles = Some(HashMap::from([(4, 90.0)]));
        assert_eq!(core, expected_pose_state);

        let follow_warm = json!([{
            "hinge": 4,
            "target_angle_deg": pose_value["angles"]["4"].as_f64().unwrap()
        }]);
        let canonical = core
            .invoke_json(&pose_solve_diagonal_canonical_request(follow_warm))
            .expect("canonical pose solve must succeed");
        let canonical_value: Value = serde_json::from_str(&canonical).expect("canonical pose JSON");
        assert_eq!(canonical_value["angles"]["4"], 90.0);
        assert_eq!(canonical_value["soft"], Value::Null);
        assert_eq!(core, expected_pose_state);
        assert_eq!(canonical, pose);

        let before_fold_all = core.clone();
        let fold_all_zero = core
            .invoke_json(&fold_all_preview_diagonal_zero_request())
            .expect("zero fold-all preview must succeed");
        let fold_zero_value: Value =
            serde_json::from_str(&fold_all_zero).expect("zero fold-all JSON");
        assert_eq!(fold_zero_value["requested_percent"], 0.0);
        assert_eq!(
            fold_zero_value["requested_angles"][0]["target_angle_deg"],
            0.0
        );
        assert_eq!(core, before_fold_all);

        let fold_all = core
            .invoke_json(&fold_all_preview_diagonal_request(
                fold_zero_value["next_warm_seed"].clone(),
            ))
            .expect("fold-all preview must succeed");
        let fold_value: Value = serde_json::from_str(&fold_all).expect("fold-all JSON");
        assert_eq!(fold_value["requested_percent"], 50.0);
        assert_eq!(
            fold_value["requested_angles"],
            json!([{
                "hinge": 4,
                "target_angle_deg": 90.0
            }])
        );
        assert_eq!(fold_value["next_warm_seed"][0]["hinge"], 4);
        assert_eq!(fold_value["layer_order"], "unavailable_without_sequence");
        assert_eq!(core, before_fold_all);

        let fold_all_without_warm = core
            .invoke_json(&fold_all_preview_diagonal_request(Value::Null))
            .expect("fold-all without warm must succeed");
        assert_eq!(fold_all_without_warm, fold_all);
        assert_eq!(core, before_fold_all);

        assert_eq!(
            pose,
            include_str!("../tests/fixtures/pose-solve-diagonal-150x100.json").trim_end()
        );
        assert_eq!(
            fold_all_zero,
            include_str!("../tests/fixtures/fold-all-preview-diagonal-0-150x100.json").trim_end()
        );
        assert_eq!(
            fold_all,
            include_str!("../tests/fixtures/fold-all-preview-diagonal-50-150x100.json").trim_end()
        );
    }

    #[test]
    fn fold_all_detection_on_transports_pairs_through_browser_wire() {
        let (mut core, percent) = fold_all_core_with_detection(true);
        let outcome = core
            .fold_all_preview(percent, None)
            .expect("検出ONでも貫通姿勢を返す");

        assert!(!outcome.self_intersection_pairs.is_empty());
        assert_eq!(
            outcome.self_intersection_pairs,
            ori3_rigid::self_intersection_pairs(&outcome.result.frame),
            "最終姿勢の面ペアを決定順のまま運ぶ"
        );
        assert!(outcome.contact_detected);
        assert!(
            outcome
                .result
                .frame
                .warnings
                .iter()
                .any(|warning| warning == ori3_rigid::PENETRATION_WARNING)
        );

        let response = core
            .invoke_json(&request(
                "fold_all_preview",
                json!({ "percent": percent, "warmSeed": null }),
            ))
            .expect("ブラウザ用fold-all応答を返す");
        let response: Value = serde_json::from_str(&response).expect("fold-all browser JSON");
        assert_eq!(
            response["self_intersection_pairs"],
            serde_json::to_value(&outcome.self_intersection_pairs).unwrap()
        );
        assert_eq!(response["contact_detected"], true);
        assert!(
            response["frame"]["warnings"]
                .as_array()
                .expect("warningsは配列")
                .iter()
                .any(|warning| warning == ori3_rigid::PENETRATION_WARNING)
        );
    }

    #[test]
    fn fold_all_detection_off_omits_pairs_and_penetration_warning_from_browser_wire() {
        let (mut core, percent) = fold_all_core_with_detection(false);
        let outcome = core
            .fold_all_preview(percent, None)
            .expect("検出OFFでも同じ貫通姿勢を返す");

        assert!(
            !ori3_rigid::self_intersection_pairs(&outcome.result.frame).is_empty(),
            "実際には貫通する標本でOFFを検査する"
        );
        assert!(outcome.self_intersection_pairs.is_empty());
        assert!(!outcome.contact_detected);
        assert!(outcome.suspect_hinges.is_empty());
        assert!(
            outcome
                .result
                .frame
                .warnings
                .iter()
                .all(|warning| warning != ori3_rigid::PENETRATION_WARNING)
        );

        let response = core
            .invoke_json(&request(
                "fold_all_preview",
                json!({ "percent": percent, "warmSeed": null }),
            ))
            .expect("検出OFFのブラウザ用fold-all応答を返す");
        let response: Value = serde_json::from_str(&response).expect("fold-all browser JSON");
        assert!(response.get("self_intersection_pairs").is_none());
        assert_eq!(response["contact_detected"], false);
        assert_eq!(response["suspect_hinges"], json!([]));
        assert!(
            response["frame"]["warnings"]
                .as_array()
                .expect("warningsは配列")
                .iter()
                .all(|warning| warning != ori3_rigid::PENETRATION_WARNING)
        );
    }

    #[test]
    fn document_view_replay_diagnostic_transports_pairs_and_clears_stale_pairs() {
        let document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let mut view = super::build_initial_document_view(&document);
        let pairs = [(3, 7), (11, 19)];

        super::attach_replay_contact_diagnostic(&mut view, &pairs);
        assert_eq!(view.self_intersection_pairs, pairs);
        assert!(view.contact_detected);
        let populated = serde_json::to_value(&view).expect("DocumentViewをJSONへ運べる");
        assert_eq!(
            populated["self_intersection_pairs"],
            json!([[3, 7], [11, 19]])
        );

        super::attach_replay_contact_diagnostic(&mut view, &[]);
        assert!(view.self_intersection_pairs.is_empty());
        assert!(!view.contact_detected);
        let cleared = serde_json::to_value(&view).expect("空のDocumentViewをJSONへ運べる");
        assert!(cleared.get("self_intersection_pairs").is_none());
    }

    #[test]
    fn pose_follow_accepts_desktop_wire_extensions_and_updates_only_warm_state() {
        let mut core = diagonal_core();
        let before = core.clone();
        let response = core
            .invoke_json(&request(
                "pose_solve",
                json!({
                    "request": {
                        "hard": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                        "preferred": null,
                        "soft": null,
                        "warmSeed": null,
                        "upTo": 0,
                        "t": 1.0,
                        "mode": "Follow",
                        "futureDesktopField": { "keptCompatible": true }
                    }
                }),
            ))
            .expect("desktop-compatible extra request field must be ignored");
        assert_eq!(
            response,
            include_str!("../tests/fixtures/pose-solve-diagonal-150x100.json").trim_end()
        );
        let mut expected = before;
        expected.pose_angles = Some(HashMap::from([(4, 90.0)]));
        assert_eq!(core, expected);

        let default_mode = core
            .invoke_json(&request(
                "pose_solve",
                json!({
                    "request": {
                        "hard": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                        "preferred": null,
                        "soft": null,
                        "warmSeed": null,
                        "upTo": 0,
                        "t": 1.0
                    }
                }),
            ))
            .expect("omitted mode must default to Follow");
        assert_eq!(default_mode, response);
        assert_eq!(core, expected);

        let before_soft = core.clone();
        let softened = core
            .pose_solve(super::PoseSolveRequest {
                hard: vec![ori3_model::Driver {
                    hinge: 4,
                    target_angle_deg: 45.0,
                }],
                preferred: Some(vec![ori3_model::Driver {
                    hinge: 4,
                    target_angle_deg: -90.0,
                }]),
                soft: Some(ori3_soft::SoftSettings {
                    enabled: true,
                    subdivision: 0,
                    stiffness: 0.7,
                    pressure: 0.2,
                    iterations: 1,
                }),
                warm_seed: Some(vec![ori3_model::Driver {
                    hinge: 4,
                    target_angle_deg: -180.0,
                }]),
                up_to: 0,
                t: f64::NAN,
                mode: Some(super::PoseSolveMode::Follow),
            })
            .expect("desktop accepts non-finite t and lets shared calculation normalize it");
        assert_eq!(softened.result.angles.get(&4), Some(&45.0));
        let mesh = softened
            .soft
            .as_ref()
            .expect("authoritative pose returns soft mesh");
        assert!(!mesh.positions.is_empty());
        assert!(!mesh.triangles.is_empty());
        assert!(!softened.contact_detected);
        let mut expected_soft = before_soft;
        expected_soft.pose_angles = Some(softened.result.angles.clone());
        assert_eq!(core, expected_soft);

        let before_invalid_warm = core.clone();
        let error = match core.pose_solve(super::PoseSolveRequest {
            hard: Vec::new(),
            preferred: None,
            soft: None,
            warm_seed: Some(vec![ori3_model::Driver {
                hinge: 4,
                target_angle_deg: f64::NAN,
            }]),
            up_to: 0,
            t: 1.0,
            mode: Some(super::PoseSolveMode::Follow),
        }) {
            Ok(_) => panic!("non-finite Follow warm seed must fail"),
            Err(error) => error,
        };
        assert_eq!(error, "追従計算の出発角に有限でない値があります");
        assert_eq!(core, before_invalid_warm);
    }

    #[test]
    fn canonical_pose_uses_document_up_to_and_t_and_ignores_live_warm() {
        let mut core = Ori3AppCore::new();
        core.invoke_json(&new_150x100_request())
            .expect("new command must succeed");
        core.invoke_json(&apply_fold_through_request())
            .expect("fold setup must succeed");

        let start = super::canonical_document_seed(&core.doc, &core.faces, 0, 1.0);
        let half = super::canonical_document_seed(&core.doc, &core.faces, 1, 0.5);
        let complete = super::canonical_document_seed(&core.doc, &core.faces, 1, 1.0);
        assert_eq!(start.get(&4), Some(&0.0));
        assert_eq!(half.get(&4), Some(&-90.0));
        assert_eq!(complete.get(&4), Some(&-180.0));

        core.pose_angles = Some(HashMap::from([(4, 77.0)]));
        let before = core.clone();
        let canonical = core
            .pose_solve(super::PoseSolveRequest {
                hard: Vec::new(),
                preferred: Some(vec![ori3_model::Driver {
                    hinge: 4,
                    target_angle_deg: -90.0,
                }]),
                soft: None,
                warm_seed: Some(vec![ori3_model::Driver {
                    hinge: 4,
                    target_angle_deg: f64::NAN,
                }]),
                up_to: 1,
                t: 0.5,
                mode: Some(super::PoseSolveMode::Canonical),
            })
            .expect("Canonical must ignore live and explicit warm seeds");
        assert_eq!(canonical.result.angles.get(&4), Some(&-90.0));
        assert!(canonical.result.closure_rms.is_finite());
        assert!(canonical.result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        }));
        let mut expected = before;
        expected.pose_angles = Some(canonical.result.angles.clone());
        assert_eq!(core, expected);
    }

    #[test]
    fn fold_all_reports_hundred_percent_local_violations_and_keeps_state_on_errors() {
        let mut core = diagonal_core();
        core.edit_apply(EditOp::AddSegment {
            a: [1.0, 0.0],
            b: [0.0, 2.0 / 3.0],
            kind: EdgeKind::Mountain,
        })
        .expect("second diagonal must succeed");
        let local = ori3_cp::local_violations(&core.doc.cp);
        assert!(!local.is_empty());
        core.pose_angles = Some(HashMap::from([(4, 42.0)]));
        let before = core.clone();

        let fifty = core
            .fold_all_preview(50.0, None)
            .expect("fifty percent must return a pose");
        assert!(fifty.flat_fold_violations.is_empty());
        assert!(
            fifty
                .requested_angles
                .windows(2)
                .all(|pair| pair[0].hinge < pair[1].hinge)
        );
        assert!(
            fifty
                .next_warm_seed
                .windows(2)
                .all(|pair| pair[0].hinge < pair[1].hinge)
        );
        assert_eq!(core, before);

        let hundred = core
            .fold_all_preview(100.0, Some(fifty.next_warm_seed.clone()))
            .expect("non-flat-foldable CP must still return a pose");
        assert!(
            local
                .iter()
                .all(|vertex| hundred.flat_fold_violations.contains(vertex))
        );
        assert!(
            hundred
                .result
                .frame
                .warnings
                .iter()
                .any(|warning| { warning == super::FOLD_ALL_FLAT_FOLD_WARNING })
        );
        assert!(hundred.result.closure_rms.is_finite());
        assert!(
            hundred
                .result
                .angles
                .values()
                .all(|angle| angle.is_finite())
        );
        assert_eq!(core, before);

        for percent in [-1.0, 100.01, f64::NAN] {
            let error = match core.fold_all_preview(percent, None) {
                Ok(_) => panic!("invalid percent must fail"),
                Err(error) => error,
            };
            assert!(
                error
                    .starts_with("全部の折り目を動かす割合は有限な0%以上100以下で指定してください")
            );
            assert_eq!(core, before);
        }

        let error = match core.fold_all_preview(
            50.0,
            Some(vec![ori3_model::Driver {
                hinge: 4,
                target_angle_deg: 181.0,
            }]),
        ) {
            Ok(_) => panic!("out-of-range warm angle must fail"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            "一時表示の出発角は有限な-180度以上180度以下で指定してください（辺ID 4: 181度）"
        );
        assert_eq!(core, before);
    }

    #[test]
    fn sequence_step_edits_match_history_soft_and_move_noop_contracts() {
        let mut core = Ori3AppCore::new();
        core.document_new(Paper {
            width_mm: 150.0,
            height_mm: 100.0,
        })
        .expect("new document must succeed");
        core.doc.display.soft_enabled = true;
        core.doc.display.soft_stiffness = 0.75;
        core.doc.display.soft_pressure = 0.25;

        let pushed = core
            .sequence_apply(json!({
                "type": "PushStep",
                "step": pose_step_value(10, "first")
            }))
            .expect("push must succeed");
        assert_eq!(core.undo_stack.len(), 1);
        assert_eq!(pushed.doc.sequence.len(), 1);
        assert_eq!(
            pushed.step_creases,
            vec![StepCreases {
                step: 10,
                lines: vec![]
            }]
        );
        assert_eq!(
            pushed.doc.sequence[0].finish_soft,
            Some(ori3_model::FinishSoftSettings {
                enabled: true,
                stiffness: 0.75,
                pressure: 0.25,
            })
        );

        let inserted = core
            .sequence_apply(json!({
                "type": "InsertStep",
                "index": 0,
                "step": pose_step_value(20, "inserted")
            }))
            .expect("insert must succeed");
        assert_eq!(core.undo_stack.len(), 2);
        assert_eq!(
            inserted
                .doc
                .sequence
                .iter()
                .map(|step| step.id)
                .collect::<Vec<_>>(),
            vec![20, 10]
        );
        assert!(inserted.doc.sequence[0].finish_soft.is_none());

        let updated = core
            .sequence_apply(json!({
                "type": "UpdateStep",
                "step": pose_step_value(20, "updated")
            }))
            .expect("update must succeed");
        assert_eq!(core.undo_stack.len(), 3);
        assert_eq!(updated.doc.sequence[0].note, "updated");
        assert!(updated.doc.sequence[0].finish_soft.is_none());

        core.pose_angles = Some(HashMap::from([(999, 42.0)]));
        let before_noop = core.clone();
        let noop = core
            .sequence_apply(json!({ "type": "MoveStep", "id": 20, "to_index": 0 }))
            .expect("same-index move must succeed");
        assert!(noop.frame.is_some());
        assert_eq!(core, before_noop);

        let moved = core
            .sequence_apply(json!({ "type": "MoveStep", "id": 10, "to_index": 0 }))
            .expect("move must succeed");
        assert_eq!(core.undo_stack.len(), 4);
        assert_eq!(
            moved
                .doc
                .sequence
                .iter()
                .map(|step| step.id)
                .collect::<Vec<_>>(),
            vec![10, 20]
        );
        assert_eq!(core.pose_angles.as_ref(), Some(&moved.angles));

        let removed = core
            .sequence_apply(json!({ "type": "RemoveStep", "id": 20 }))
            .expect("remove must succeed");
        assert_eq!(core.undo_stack.len(), 5);
        assert_eq!(removed.doc.sequence.len(), 1);
        assert_eq!(removed.doc.sequence[0].id, 10);
        assert_eq!(removed.step_creases.len(), 1);
        assert_eq!(removed.step_creases[0].step, 10);
        assert_eq!(super::next_step_id(&core.doc, &core.step_creases), 21);

        for (op, expected) in [
            (
                json!({
                    "type": "InsertStep",
                    "index": 2,
                    "step": pose_step_value(30, "invalid")
                }),
                "挿入位置 2 が手順の数を超えています",
            ),
            (
                json!({ "type": "RemoveStep", "id": 999 }),
                "手順ID 999 が見つかりません",
            ),
            (
                json!({
                    "type": "UpdateStep",
                    "step": pose_step_value(999, "invalid")
                }),
                "手順ID 999 が見つかりません",
            ),
            (
                json!({ "type": "MoveStep", "id": 10, "to_index": 1 }),
                "移動先 1 が手順の数を超えています",
            ),
        ] {
            let before = core.clone();
            let error = match core.sequence_apply(op) {
                Ok(_) => panic!("invalid edit must fail"),
                Err(error) => error,
            };
            assert_eq!(error, expected);
            assert_eq!(core, before);
        }

        core.doc.sequence.push(core.doc.sequence[0].clone());
        let before_duplicate = core.clone();
        let error =
            match core.sequence_apply(json!({ "type": "MoveStep", "id": 10, "to_index": 0 })) {
                Ok(_) => panic!("duplicate IDs must fail before move resolution"),
                Err(error) => error,
            };
        assert_eq!(error, "同じ折り手順が二重に入っています");
        assert_eq!(core, before_duplicate);
    }

    #[test]
    fn sequence_parser_keeps_spatial_hit_and_allows_the_frontend_envelope() {
        let value = json!({
            "type": "PreviewFoldThrough",
            "up_to": 0,
            "line": [[0.0, 0.0], [1.0, 2.0 / 3.0]],
            "keep_side_point": [0.0, 2.0 / 3.0],
            "target_layers": null,
            "target_pleat_count": 2,
            "direction": "Up",
            "spatial": {
                "from": [0.5, 0.25, -0.25],
                "to": [0.5, 0.5, -0.25],
                "grab_face": 1,
                "mode": "flap"
            }
        });
        let (operation, spatial) =
            super::parse_sequence_operation(value).expect("frontend envelope must parse");
        assert!(matches!(
            operation,
            ori3_model::SeqOp::PreviewFoldThrough { up_to: 0, .. }
        ));
        let spatial = spatial.expect("spatial hit must be retained");
        assert_eq!(spatial.from, [0.5, 0.25, -0.25]);
        assert_eq!(spatial.to, [0.5, 0.5, -0.25]);
        assert_eq!(spatial.grab_face, 1);

        let (_, camel_case) = super::parse_sequence_operation(json!({
            "type": "PreviewFoldThrough",
            "up_to": 0,
            "line": [[0.0, 0.0], [1.0, 2.0 / 3.0]],
            "keep_side_point": [0.0, 2.0 / 3.0],
            "target_layers": null,
            "direction": "Up",
            "spatial": {
                "from": [0.0, 0.0, 0.0],
                "to": [1.0, 0.0, 0.0],
                "grabFace": 7
            }
        }))
        .expect("legacy grabFace alias must parse");
        assert_eq!(camel_case.unwrap().grab_face, 7);

        let error = match super::parse_sequence_operation(json!({
            "type": "MoveStep",
            "id": 1,
            "to_index": 0,
            "spatial": {
                "from": [0.0, 0.0, 0.0],
                "to": [1.0, 0.0, 0.0],
                "grab_face": 0
            }
        })) {
            Ok(_) => panic!("MoveStep must retain its strict payload"),
            Err(error) => error,
        };
        assert_eq!(error, "折る操作を読み取れませんでした");

        let mut core = Ori3AppCore::new();
        for unsupported in [
            json!({
                "type": "PreviewFoldTargets",
                "up_to": 0,
                "line": [[0.0, 0.0], [1.0, 1.0]],
                "keep_side_point": [0.0, 1.0]
            }),
            json!({
                "type": "CreaseOnlyTop",
                "up_to": 0,
                "material_line": [[0.0, 0.0], [1.0, 1.0]],
                "material_keep_side_point": [0.0, 1.0],
                "direction": "Up"
            }),
            json!({
                "type": "PreviewFoldTargetsOnMaterial",
                "up_to": 0,
                "material_line": [[0.0, 0.0], [1.0, 1.0]],
                "material_keep_side_point": [0.0, 1.0]
            }),
        ] {
            let before = core.clone();
            let error = core
                .invoke_json(&request("sequence_apply", json!({ "op": unsupported })))
                .expect_err("desktop-unsupported variant must be explicit Err");
            assert_eq!(error, "折る操作を読み取れませんでした");
            assert_eq!(core, before);
        }
    }

    #[test]
    fn flat_motion_and_technique_use_shared_layer_calculations_and_one_undo_each() {
        let mut motion_core = Ori3AppCore::new();
        motion_core
            .document_new(Paper {
                width_mm: 150.0,
                height_mm: 150.0,
            })
            .expect("new document must succeed");
        motion_core
            .sequence_apply(json!({
                "type": "FoldThrough",
                "up_to": 0,
                "line": [[0.5, 0.0], [0.5, 1.0]],
                "keep_side_point": [0.25, 0.5],
                "target_layers": null,
                "direction": "Up",
                "accept_additional_crease": false
            }))
            .expect("setup fold must succeed");
        let (before_motion, _) = ori3_layers::flat_state_at(
            &motion_core.doc,
            &motion_core.faces,
            motion_core.doc.sequence.len(),
        )
        .expect("setup must be flat");
        assert_eq!(before_motion.order.len(), 2);
        let bottom = before_motion.order[0];
        let expected_order = vec![before_motion.order[1], bottom];
        let cp_before = motion_core.doc.cp.clone();

        let motion = ori3_model::SeqOp::FlatMotion {
            up_to: 1,
            parts: vec![ori3_model::MotionPart {
                layers: vec![bottom],
                region: Vec::new(),
                transform: ori3_model::MotionTransform::Stay,
                turn: ori3_model::LayerTurn::Outside(ori3_model::FoldDirection::Up),
                reverse_layers: None,
            }],
            kind: TechniqueKind::Pose,
        };
        let moved = motion_core
            .sequence_apply(serde_json::to_value(motion).expect("FlatMotion JSON"))
            .expect("FlatMotion must succeed");
        assert_eq!(motion_core.undo_stack.len(), 2);
        assert_eq!(moved.doc.sequence.len(), 2);
        assert_eq!(moved.doc.sequence[1].kind, TechniqueKind::Pose);
        assert_eq!(moved.doc.cp.edges.len(), cp_before.edges.len());
        assert_eq!(moved.doc.cp.vertices.len(), cp_before.vertices.len());
        assert!(moved.frame.is_some());
        assert!(moved.skipped.is_empty());
        let (after_motion, _) = ori3_layers::flat_state_at(
            &motion_core.doc,
            &motion_core.faces,
            motion_core.doc.sequence.len(),
        )
        .expect("FlatMotion result must replay flat");
        assert_eq!(after_motion.order, expected_order);

        let mut technique_core = Ori3AppCore::new();
        technique_core
            .document_new(Paper {
                width_mm: 150.0,
                height_mm: 150.0,
            })
            .expect("new document must succeed");
        let technique = technique_core
            .sequence_apply(json!({
                "type": "Technique",
                "up_to": 0,
                "kind": "Pleat",
                "flap": [],
                "line": [[0.4, 0.0], [0.4, 1.0]],
                "reference_point": [0.5, 0.5]
            }))
            .expect("pleat must succeed");
        assert_eq!(technique_core.undo_stack.len(), 1);
        assert_eq!(technique.faces.len(), 3);
        assert_eq!(technique.doc.sequence.len(), 1);
        assert_eq!(technique.doc.sequence[0].kind, TechniqueKind::Pleat);
        assert_eq!(technique.doc.sequence[0].drivers.len(), 2);
        assert_eq!(
            technique.doc.sequence[0].layer_order.as_ref().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            technique
                .doc
                .cp
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Mountain)
                .count(),
            1
        );
        assert_eq!(
            technique
                .doc
                .cp
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Valley)
                .count(),
            1
        );
        assert!(technique.frame.is_some());
        assert!(technique.skipped.is_empty());

        let undone = technique_core.edit_undo().expect("undo must succeed");
        assert!(undone.doc.sequence.is_empty());
        assert_eq!(undone.faces.len(), 1);

        let before_invalid = technique_core.clone();
        let error = match technique_core.sequence_apply(json!({
            "type": "Technique",
            "up_to": 0,
            "kind": "Pose",
            "flap": [],
            "line": [[0.4, 0.0], [0.4, 1.0]],
            "reference_point": [0.5, 0.5]
        })) {
            Ok(_) => panic!("unsupported technique must fail"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            "この折り方はまだ選べません。手動の折り操作で代替してください"
        );
        assert_eq!(technique_core, before_invalid);

        let before_unfoldable = technique_core.clone();
        let error = match technique_core.sequence_apply(json!({
            "type": "FoldThrough",
            "up_to": 0,
            "line": [[2.0, 0.0], [2.0, 1.0]],
            "keep_side_point": [1.5, 0.5],
            "target_layers": null,
            "direction": "Up",
            "accept_additional_crease": false
        })) {
            Ok(_) => panic!("unfoldable line must fail"),
            Err(error) => error,
        };
        assert!(error.contains("折り線"));
        assert_eq!(technique_core, before_unfoldable);
    }

    #[test]
    fn sequence_fold_through_and_half_replay_match_complete_fixtures() {
        let mut core = Ori3AppCore::new();
        let initial = core
            .invoke_json(&new_150x100_request())
            .expect("new command must succeed");
        let before_preview = core.clone();

        let preview = core
            .invoke_json(&preview_fold_through_request())
            .expect("preview must succeed");
        assert_eq!(core, before_preview);

        let applied = core
            .invoke_json(&apply_fold_through_request())
            .expect("fold must succeed");
        assert_eq!(core.undo_stack.len(), 1);
        assert!(core.redo_stack.is_empty());
        assert!(core.dirty);
        let applied_value: Value = serde_json::from_str(&applied).expect("apply JSON");
        assert_eq!(
            applied_value["doc"]["sequence"].as_array().unwrap().len(),
            1
        );
        assert_eq!(applied_value["step_creases"].as_array().unwrap().len(), 1);
        assert_eq!(applied_value["sequence_targets"][0]["hinge"], 4);
        assert_eq!(
            applied_value["sequence_targets"][0]["target_angle_deg"],
            -180.0
        );
        assert_eq!(applied_value["angles"]["4"], -180.0);
        assert_eq!(core.pose_angles.as_ref().unwrap().get(&4), Some(&-180.0));

        let replay = core
            .invoke_json(&replay_fold_through_half_request())
            .expect("half replay must succeed");
        let replay_value: Value = serde_json::from_str(&replay).expect("replay JSON");
        assert_eq!(replay_value["sequence_targets"][0]["hinge"], 4);
        assert_eq!(
            replay_value["sequence_targets"][0]["target_angle_deg"],
            -90.0
        );
        assert_eq!(replay_value["angles"]["4"], -90.0);
        assert_eq!(core.pose_angles.as_ref().unwrap().get(&4), Some(&-90.0));

        let before_soft = core.clone();
        let softened = core
            .invoke_json(&request(
                "sequence_replay",
                json!({
                    "upTo": 1,
                    "t": 0.5,
                    "soft": {
                        "enabled": true,
                        "subdivision": 0,
                        "stiffness": 0.7,
                        "pressure": 0.2,
                        "iterations": 1
                    }
                }),
            ))
            .expect("soft replay must succeed");
        let softened: Value = serde_json::from_str(&softened).expect("soft replay JSON");
        let soft = softened["soft"]
            .as_object()
            .expect("soft mesh must be returned");
        assert!(!soft["positions"].as_array().unwrap().is_empty());
        assert!(!soft["triangles"].as_array().unwrap().is_empty());
        assert_eq!(softened["sequence_targets"][0]["target_angle_deg"], -90.0);
        assert_eq!(softened["angles"]["4"], -90.0);
        assert_eq!(core, before_soft);

        let undone = core.edit_undo().expect("undo must succeed");
        assert_eq!(serde_json::to_string(&undone).unwrap(), initial);
        let redone = core.edit_redo().expect("redo must succeed");
        assert_eq!(serde_json::to_string(&redone).unwrap(), applied);

        assert_eq!(
            preview,
            include_str!("../tests/fixtures/sequence-preview-fold-through-150x100.json").trim_end()
        );
        assert_eq!(
            applied,
            include_str!("../tests/fixtures/sequence-apply-fold-through-150x100.json").trim_end()
        );
        assert_eq!(
            replay,
            include_str!("../tests/fixtures/sequence-replay-fold-through-half-150x100.json")
                .trim_end()
        );
    }
}
