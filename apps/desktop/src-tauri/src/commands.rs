//! IPCコマンド層: 各コマンドはDocumentStoreへ委譲するだけの薄いラッパー。
//! 全コマンドをpanic捕捉ラッパー`guard`で包み、アプリを落とさない(SYS-005)。
//! 全コマンドを`#[tauri::command(async)]`にしてスレッドプールで実行する
//! (同期fnはメインスレッド実行になり、validate等の計算でUIが引っかかるため)。
//!
//! 設計規約: ロック中に重い計算をしない(pose_solveなど他コマンドを待たせないため)。
//! ロック下ではstoreの状態更新と複製だけを行い、手順の再生や姿勢計算は
//! ロックを解放してから実行する(`view_command` / `pose_solve` / `sequence_replay`)。

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::autosave;
use crate::store::{
    DocumentStore, DocumentView, FoldImportError, SpatialFoldSpec,
    add_penetration_warning_for_intersections, apply_layer_order_display_settings, attach_replay,
    filter_penetration_warnings, flat_fold_notice_violations, frame_surface_rank_order,
    pose_flat_fold_notice_intersects, prevent_replay_overlap_if_authoritative,
    replay_flat_fold_notice_violations, replay_surface_rank_order,
};
use ori3_export::fold::{
    FoldExport, FoldIssue, document_to_fold, fold_to_document, parse_fold_1_2, write_fold_1_2,
};
use ori3_export::{CpSvgOptions, cp_png, cp_svg, diagram_pdf, diagram_svg_pages};
use ori3_model::{
    CreasePattern, DisplaySettings, Document, Driver, EdgeId, EdgeKind, EditOp, FaceId,
    FinishSoftSettings, FoldStep, Frame3D, Paper, SeqOp, TechniqueKind, VertexId,
};
use ori3_propose::{
    CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, LeafSite, Packing,
    PoseScan, SearchAbort, SearchBudget, SearchCancellation, SearchControl, SearchWatchdog,
    Skeleton, TipSite, VerifiedPlan, body_on_paper, generate, pack,
    search_to_completion_with_control, verify_search_completion,
};
use ori3_soft::{SoftMesh, SoftSettings};

/// 複数ファイル書き出し用の同名一時ファイルを区別する連番。
static EXPORT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
type ExportPayloads = Vec<(String, Vec<u8>)>;
type ExportBuild = (ExportPayloads, Vec<FoldIssue>);

/// たわみの計算結果を足した `pose_solve` の戻り値(SIM-012)。
/// 既存の中身は `#[serde(flatten)]` でそのまま並べるので、たわみを使わない
/// 画面から見ると今までと同じ形のまま(`soft` が `null` で増えるだけ)。
#[derive(Serialize)]
pub struct PoseOutcome {
    #[serde(flatten)]
    pub result: ori3_rigid::SolveResult,
    pub soft: Option<SoftMesh>,
    pub suspect_hinges: Vec<EdgeId>,
    /// 紙どうしの接触を検出したか。接触しても要求角まで計算を続ける。
    pub contact_detected: bool,
    /// 今回の±180°指定に関係し、指定角まで届かなかったか紙が食い込んだ通知対象の点。
    pub flat_fold_violations: Vec<VertexId>,
}

/// 通常の連続操作と、Documentから同じ姿勢を再導出する確定操作を区別する。
///
/// IPCで省略された場合も既存呼出しと同じ [`Self::Follow`] にする。
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub enum PoseSolveMode {
    #[default]
    Follow,
    Canonical,
}

/// 画面から姿勢計算へ渡す値。Tauriの1引数へまとめるだけで、各値の意味は変えない。
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

/// IPC境界の省略可能なmodeを確定した、姿勢計算本体への入力。
pub(crate) struct PoseSolveInput {
    pub(crate) hard: Vec<Driver>,
    pub(crate) preferred: Option<Vec<Driver>>,
    pub(crate) soft: Option<SoftSettings>,
    pub(crate) warm_seed: Option<Vec<Driver>>,
    pub(crate) up_to: usize,
    pub(crate) t: f64,
    pub(crate) mode: PoseSolveMode,
}

/// 手順を持たない一斉折りでは、物理的な面の上下を確定できないことを示す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldAllLayerOrder {
    UnavailableWithoutSequence,
}

const FOLD_ALL_FLAT_FOLD_WARNING: &str =
    "平らにたためない折り目の集まりがあります。表示できた形を返しています";

/// 全折り目を同じ割合で動かす、一時表示専用コマンドの戻り値。
///
/// `Document`や`FoldStep`を含めず、通常姿勢のcache・保存・Undoも更新しない。
#[derive(Serialize)]
pub struct FoldAllPreviewOutcome {
    #[serde(flatten)]
    pub result: ori3_rigid::SolveResult,
    /// 利用者が要求した0〜100の割合。
    pub requested_percent: f64,
    /// 有効な山谷ヒンジへ渡した希望角（辺ID昇順）。
    pub requested_angles: Vec<Driver>,
    /// 次の割合を連続して解くとき、そのまま入力へ戻せる実角（辺ID昇順）。
    pub next_warm_seed: Vec<Driver>,
    /// 最終姿勢で交差に関係した可能性のある折り目。
    pub suspect_hinges: Vec<EdgeId>,
    /// 経路上または最終姿勢で紙どうしの接触を検出したか。
    pub contact_detected: bool,
    /// 100%要求で、角度未到達または最終交差に関係する通知対象の点。
    pub flat_fold_violations: Vec<VertexId>,
    /// 手順がないため重なり順を返さない、という必須の構造化状態。
    pub layer_order: FoldAllLayerOrder,
}

/// たわみの計算結果を足した `sequence_replay` の戻り値(SIM-012)。
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
    pub contact_detected: bool,
    pub flat_fold_violations: Vec<VertexId>,
}

/// たわみの網を作る。指定が無い・切ってあるときは何もしない(従来どおりの動作)。
///
/// 設計規約: 重い計算なので必ずロックの外から呼ぶこと。
fn soft_mesh(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
    frame: &ori3_model::Frame3D,
    surface_order_authoritative: bool,
    soft: Option<&SoftSettings>,
) -> Option<SoftMesh> {
    let settings = soft?;
    if !settings.enabled || !surface_order_authoritative {
        return None;
    }
    // softの接触・袋・三角形層も、画面と同じ幾何由来の順位を使う。
    // 論理的な保存順である`layer`は返却frame上では変えず、soft入力だけを複製する。
    frame_surface_rank_order(frame)?;
    let mut display_frame = frame.clone();
    for face in &mut display_frame.faces {
        face.layer = face.surface_rank;
    }
    Some(ori3_soft::relax(cp, faces, &display_frame, settings))
}

/// 新しく確定するPoseだけへ、その時点のたわみ3値を記録する。
///
/// Insert/Updateは旧手順の並べ替え・注釈更新にも使うため、欠けた値を現在値で
/// 補わない。既に記録済みの値も上書きしない。
fn record_finish_soft(operation: &mut SeqOp, display: &DisplaySettings) {
    let SeqOp::PushStep { step } = operation else {
        return;
    };
    if step.kind == TechniqueKind::Pose && step.finish_soft.is_none() {
        step.finish_soft = Some(FinishSoftSettings::from(display));
    }
}

/// 保存した3値だけを適用し、計算用の細分数・反復数は呼び出し値のまま保つ。
fn recorded_soft_settings(
    document: &Document,
    up_to: usize,
    t: f64,
    live: Option<SoftSettings>,
) -> Option<SoftSettings> {
    let Some(recorded) = document.finish_soft_at(up_to, t) else {
        // 全Poseに記録がない旧作品は、従来どおりDisplay由来のIPC値を使う。
        return live;
    };
    let mut settings = live.unwrap_or_default();
    settings.enabled = recorded.enabled;
    settings.stiffness = recorded.stiffness;
    settings.pressure = recorded.pressure;
    Some(settings)
}

/// 過去・途中位置は保存値を再現し、最新終点だけは次の仕上げを作るライブ調整を優先する。
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

/// 保存手順から解決した完全な下→上順だけを、後続手順用の論理層へ刻印する。
///
/// 表示用`surface_rank`はrigidが現在の幾何から求めた値を維持する。不完全・重複・
/// 別CP由来の順序ならフレームを一切変えない。
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
        let rank = ranks[&face.face];
        face.layer = rank;
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

/// 全値finiteの最良候補はそのまま表示し、数値が壊れた場合だけ直前形へ戻す。
fn fallback_nonfinite_pose(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
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
            .push("追従計算が収束していません".to_string());
    }
    previous
}

/// panicをErr文字列に変換する(SYS-005: アプリを落とさない)。
fn guard<T>(f: impl FnOnce() -> Result<T, String> + std::panic::UnwindSafe) -> Result<T, String> {
    std::panic::catch_unwind(f).unwrap_or_else(|payload| {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "詳細不明".to_string()
        };
        Err(format!("内部エラーが発生しました: {msg}"))
    })
}

/// storeのロックを取る。過去のpanicで毒化されていても中身を取り出して続行する
/// (storeは「複製に適用→確定」方式のため、panic時も直前の整合状態を保っている)。
fn lock_inner(state: &Mutex<DocumentStore>) -> MutexGuard<'_, DocumentStore> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock<'a>(state: &'a State<'_, Mutex<DocumentStore>>) -> MutexGuard<'a, DocumentStore> {
    lock_inner(state.inner())
}

/// DocumentViewを返す操作の共通後処理: 手順を最新ステップまで自動再生して
/// 立体・飛ばした手順・警告をビューへ載せる(SEQ-004)。
///
/// 設計規約: ロック中に重い計算をしない。`f` の中で取ったロックは `f` を抜けた時点で
/// 解放されているので、再生(面400・10手順でrelease約23ms)はロックの外で走る。
fn store_view_pose_angles(state: &Mutex<DocumentStore>, view: &DocumentView) {
    if view.frame.is_some() && view.angles.values().all(|angle| angle.is_finite()) {
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .store_pose_angles(view.angles.clone());
    }
}

fn view_command(
    state: &State<'_, Mutex<DocumentStore>>,
    f: impl FnOnce() -> Result<DocumentView, String>,
) -> Result<DocumentView, String> {
    let mut view = f()?; // ここでロックは解放済み
    attach_replay(&mut view);
    store_view_pose_angles(state.inner(), &view);
    Ok(view)
}

#[tauri::command(async)]
pub fn document_new(
    state: State<'_, Mutex<DocumentStore>>,
    paper: Paper,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(&state, || lock(&state).new_document(paper))
    }))
}

#[tauri::command(async)]
pub fn document_open(
    state: State<'_, Mutex<DocumentStore>>,
    path: String,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        let path = Path::new(&path);
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fold"))
        {
            // I/O・parse・変換はstore lockの外で終え、候補値を組み立ててから
            // storeの既存commit境界へ渡す。失敗時はstoreへ一度も触れない。
            let source = std::fs::read_to_string(path)
                .map_err(|error| format!("ファイルを開けませんでした: {error}"))?;
            let file = parse_fold_1_2(&source)
                .map_err(|error| FoldImportError::Parse(error).to_string())?;
            let import = fold_to_document(&file)
                .map_err(|error| FoldImportError::Conversion(error).to_string())?;
            view_command(&state, || Ok(lock(&state).import_fold(import)))
        } else {
            view_command(&state, || lock(&state).open(path))
        }
    }))
}

#[tauri::command(async)]
pub fn document_save(
    app: tauri::AppHandle,
    state: State<'_, Mutex<DocumentStore>>,
    path: Option<String>,
) -> Result<(), String> {
    guard(AssertUnwindSafe(|| {
        let mut store = lock(&state);
        store.save(path.as_deref().map(Path::new))?;
        let document_path = store.current_path();
        drop(store);
        // 保存できた内容は自動保存から復元する必要がない(SYS-003)
        if let Ok(dir) = autosave::app_data_dir(&app) {
            autosave::discard(&dir, document_path.as_deref());
        }
        Ok(())
    }))
}

/// 前回の異常終了で残った自動保存があるか調べる(SYS-003)。
/// あればその情報を返し、フロントが復旧ダイアログで復元するか尋ねる。
#[tauri::command(async)]
pub fn recovery_check(app: tauri::AppHandle) -> Result<Option<autosave::RecoveryInfo>, String> {
    guard(AssertUnwindSafe(|| {
        let dir = autosave::app_data_dir(&app)?;
        Ok(autosave::check(&dir))
    }))
}

/// 復旧ダイアログの答えを実行する。`accept`なら自動保存の内容を現在の作品にし、
/// そうでなければ自動保存ファイルを消す(以後は提案しない)。
///
/// 設計規約: 読み込みとJSON解釈はロックの外、状態の入れ替えだけロック下で行う。
#[tauri::command(async)]
pub fn recovery_restore(
    app: tauri::AppHandle,
    state: State<'_, Mutex<DocumentStore>>,
    accept: bool,
) -> Result<Option<DocumentView>, String> {
    guard(AssertUnwindSafe(|| {
        let dir = autosave::app_data_dir(&app)?;
        if !accept {
            let document_path = lock(&state).current_path();
            autosave::discard(&dir, document_path.as_deref());
            return Ok(None);
        }
        let Some(mut view) = autosave::restore(&state, &dir)? else {
            return Ok(None);
        };
        attach_replay(&mut view); // 重い再生はロック解放後(view_commandと同じ規約)
        store_view_pose_angles(state.inner(), &view);
        Ok(Some(view))
    }))
}

#[tauri::command(async)]
pub fn edit_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: EditOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(&state, || lock(&state).apply_edit(op))
    }))
}

/// 画面での1回の入力から生じた複数の編集を、元に戻す1回で戻せるようにまとめて適用する。
/// 曲線1本(折れ線の全区間と曲がるための線)や、左右対称で増える鏡像の線が対象。
#[tauri::command(async)]
pub fn edit_apply_batch(
    state: State<'_, Mutex<DocumentStore>>,
    ops: Vec<EditOp>,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(move || {
        let mut ops = Some(ops);
        view_command(&state, || {
            lock(&state).apply_edits(ops.take().expect("1回だけ呼ばれる"))
        })
    }))
}

#[tauri::command(async)]
pub fn edit_undo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(&state, || lock(&state).undo())
    }))
}

#[tauri::command(async)]
pub fn edit_redo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(&state, || lock(&state).redo())
    }))
}

#[derive(Deserialize)]
struct SpatialEnvelope {
    #[serde(default)]
    spatial: Option<SpatialFoldSpec>,
}

fn parse_sequence_operation(
    value: serde_json::Value,
) -> Result<(SeqOp, Option<SpatialFoldSpec>), String> {
    let spatial = serde_json::from_value::<SpatialEnvelope>(value.clone())
        .map_err(|_| "折る位置を読み取れませんでした".to_string())?
        .spatial;
    let operation = serde_json::from_value::<SeqOp>(value)
        .map_err(|_| "折る操作を読み取れませんでした".to_string())?;
    Ok((operation, spatial))
}

#[tauri::command(async)]
pub fn sequence_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: serde_json::Value,
) -> Result<DocumentView, String> {
    apply_sequence_operation_transactionally(state.inner(), op)
}

/// JSONの読み取りから候補viewの導出・確定・返却までを1経路にまとめる。
///
/// MoveStepはstoreが候補Documentの再生結果を確定前に導出して返すため、`frame`が
/// 既にあるviewを再導出しない。これにより、導出失敗後にDocumentだけが確定する
/// 時間差を作らない。この関数はTauriの`State`に依存せず、command契約検査も本番と
/// 同じtransaction経路を直接通す。
pub(crate) fn apply_sequence_operation_transactionally(
    state: &Mutex<DocumentStore>,
    op: serde_json::Value,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        let (mut operation, spatial) = parse_sequence_operation(op)?;
        let is_move_step = matches!(&operation, SeqOp::MoveStep { .. });
        let (mut view, move_step_noop) = {
            let mut store = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let document = store.export_inputs();
            record_finish_soft(&mut operation, &document.display);
            let view = store.apply_seq_with_spatial(operation, spatial)?;
            let move_step_noop = is_move_step && view.doc == document;
            (view, move_step_noop)
        };
        if view.frame.is_none() {
            attach_replay(&mut view);
        }
        // 同一位置MoveStepはDocumentだけでなくpose_anglesを含むstore全体がno-op。
        // replay済みviewを返すだけで、通常commandのwarm-start保存も行わない。
        if !move_step_noop {
            store_view_pose_angles(state, &view);
        }
        Ok(view)
    }))
}

/// 既存の2設定を、警告だけの検出と明示的な形状補正へ対応付ける。
fn pose_motion_contact_options(
    overlap_enabled: bool,
    penetration_enabled: bool,
) -> ori3_rigid::MotionContactOptions {
    ori3_rigid::MotionContactOptions {
        detect: penetration_enabled,
        prevent: overlap_enabled,
    }
}

/// Canonical候補へ渡すDocument由来seedを、全折りヒンジを含む形へ正規化する。
/// 手順のない書類は全0度、手順のある書類は再生角を上書きし、不足hingeは0度にする。
fn canonical_document_seed(
    doc: &Document,
    faces: &[ori3_cp::Face],
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

/// 折り角度の追従計算(Task 1-8)。driver角を固定して残りのヒンジ角を解き、
/// 3D表示用フレームを返す。前回解はstoreが保持し、warm startとして使う。
/// facesは編集時に導出済みのstoreのキャッシュを流用する(extract_faces再実行なし)。
///
/// `hard` は**厳密に固定**する折り線(いま操作しているヒンジ)、`preferred` は
/// **なるべく保ちたい目標**(以前に指定した折り線)。内部頂点のまわりでは折り角
/// どうしに拘束があるので、指定済みを全部固定すると閉包が破れて面が離れる
/// (=紙が切れて見える)。`keep` があるときは「閉包を満たす形のうち目標に
/// いちばん近いもの」を解き([`ori3_rigid::solve_near`])、紙がつながったまま
/// 以前の指定へ追従させる。`warm_seed` は初期値であり、固定条件にはしない。
///
/// 設計規約: ロック中に重い計算をしない(将来の自動保存スレッドとの共存のため)。
/// ロック下ではCP・faces・前回解の複製だけを行って即ロックを解放し、
/// solveはロックの外で実行し、結果の角度だけを短いロックで書き戻す。
#[tauri::command(async)]
pub fn pose_solve(
    state: State<'_, Mutex<DocumentStore>>,
    request: PoseSolveRequest,
) -> Result<PoseOutcome, String> {
    let PoseSolveRequest {
        hard,
        preferred,
        soft,
        warm_seed,
        up_to,
        t,
        mode,
    } = request;
    match mode.unwrap_or_default() {
        PoseSolveMode::Follow => {
            pose_solve_core(state.inner(), hard, preferred, soft, warm_seed, up_to, t)
        }
        PoseSolveMode::Canonical => pose_solve_core_with_mode(
            state.inner(),
            PoseSolveInput {
                hard,
                preferred,
                soft,
                warm_seed,
                up_to,
                t,
                mode: PoseSolveMode::Canonical,
            },
        ),
    }
}

/// Tauriの状態包装と姿勢計算を分ける。製品commandと恒久検査が同じ本体を通り、
/// 検査だけのsolver模倣を作らないための境界で、計算・cacheの契約は変えない。
pub(crate) fn pose_solve_core(
    state: &Mutex<DocumentStore>,
    hard: Vec<Driver>,
    preferred: Option<Vec<Driver>>,
    soft: Option<SoftSettings>,
    warm_seed: Option<Vec<Driver>>,
    up_to: usize,
    t: f64,
) -> Result<PoseOutcome, String> {
    pose_solve_core_with_mode(
        state,
        PoseSolveInput {
            hard,
            preferred,
            soft,
            warm_seed,
            up_to,
            t,
            mode: PoseSolveMode::Follow,
        },
    )
}

/// [`pose_solve_core`] の互換経路と、確定時のcanonical再導出を同じ後処理へ通す。
pub(crate) fn pose_solve_core_with_mode(
    state: &Mutex<DocumentStore>,
    input: PoseSolveInput,
) -> Result<PoseOutcome, String> {
    let PoseSolveInput {
        hard,
        preferred,
        soft,
        warm_seed,
        up_to,
        t,
        mode,
    } = input;
    guard(AssertUnwindSafe(|| {
        let (doc, faces, stored_warm, overlap_enabled, penetration_enabled) =
            lock_inner(state).pose_inputs(); // 複製のみ、即ロック解放
        let soft = display_soft_settings(&doc, up_to, t, soft);
        let cp = &doc.cp;
        let saved_order = ori3_layers::saved_layer_order_at(&doc, &faces, up_to, t);
        let preferred = preferred.unwrap_or_default();
        // 同じ辺が両方にあれば、現在操作中のhardを後から入れて優先する。
        // warm_seedは出発角であって要求ではないため含めない。
        let requested_targets: Vec<Driver> = preferred.iter().chain(&hard).cloned().collect();
        // CanonicalはDocumentと希望値だけから候補を作る。呼出し元やstoreに残る
        // warmは値の検査にも使わず、非有限結果のfallbackにも渡さない。
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
            return Err("追従計算の出発角に有限でない値があります".to_string());
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
                .map(|d| (d.hinge, d.target_angle_deg))
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
        // 接触補正は現在の幾何から求めたcanonical順だけをauthorityにする。
        // proofが無い場合は保存layerやFaceId列へfallbackせず、補正自体を行わない。
        // 共有網頂点へ補正するので折り目は切れない。
        let mut fallback_order: Vec<ori3_model::FaceId> =
            faces.iter().map(|face| face.id).collect();
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
        // 接触補正後も表示rankは幾何由来のまま保ち、保存順は後続手順用layerだけへ刻む。
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
        ); // SIM-007
        // たわみもロックの外で計算する(規約どおり)
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
            lock_inner(state).store_pose_angles(result.angles.clone()); // 短いロックで書き戻し
        }
        Ok(PoseOutcome {
            result,
            soft: mesh,
            suspect_hinges,
            contact_detected,
            flat_fold_violations,
        })
    }))
}

fn fold_all_preview_outcome(
    doc: &Document,
    faces: &[ori3_cp::Face],
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
    } = ori3_rigid::solve_fold_all_preview(&doc.cp, faces, percent, warm.as_ref())
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
    let intersections = ori3_rigid::self_intersection_pairs(&result.frame);
    let contact_detected = motion.contact_detected || !intersections.is_empty();
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
        flat_fold_violations.extend(ori3_cp::local_violations(&doc.cp));
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
            .push(FOLD_ALL_FLAT_FOLD_WARNING.to_string());
    }
    Ok(FoldAllPreviewOutcome {
        result,
        requested_percent,
        requested_angles,
        next_warm_seed,
        suspect_hinges,
        contact_detected,
        flat_fold_violations,
        layer_order: FoldAllLayerOrder::UnavailableWithoutSequence,
    })
}

/// 全ての有効な山谷ヒンジを0〜100%で同時に動かし、一時姿勢だけを返す。
///
/// ロック下では展開図と導出済み面を複製するだけ。手順、保存内容、Undo、通常の
/// `pose_angles`を変更しない。不収束・平坦条件違反・貫通は結果内の診断として返し、
/// コマンドを失敗させない。
#[tauri::command(async)]
pub fn fold_all_preview(
    state: State<'_, Mutex<DocumentStore>>,
    percent: f64,
    warm_seed: Option<Vec<Driver>>,
) -> Result<FoldAllPreviewOutcome, String> {
    guard(AssertUnwindSafe(|| {
        let (doc, faces) = lock(&state).replay_inputs();
        fold_all_preview_outcome(&doc, &faces, percent, warm_seed)
    }))
}

/// 手順の再生(Task 2-3)。展開図と手順列から `up_to` ステップ目(補間係数 `t`)の
/// 立体を求め直す。3D状態は保存しないので、展開図を編集した後でも再生できる。
///
/// 設計規約: ロック中に重い計算をしない。ロック下ではDocumentと導出済みfacesの複製
/// だけを行って即ロックを解放し、再生はロックの外で実行する。実角は次回の
/// warm startとしてだけ短いロックで保持し、作品ファイルには保存しない。
#[tauri::command(async)]
pub fn sequence_replay(
    state: State<'_, Mutex<DocumentStore>>,
    up_to: usize,
    t: f64,
    soft: Option<SoftSettings>,
) -> Result<ReplayOutcome, String> {
    guard(AssertUnwindSafe(|| {
        let (doc, faces) = lock(&state).replay_inputs(); // 複製のみ、即ロック解放
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
        // 検出と補正は独立した設定である。両方が有効な場合も、補正で消える前の
        // 利用者指定の姿勢を診断し、その結果を警告と原因候補へ残す。
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
        // 折る途中(t<1)は立体になるので、紙が食い込んでいないかを見る(SIM-007)。
        // 画面のバッジは ReplayResult.warnings を見るので両方へ同じ文言を載せる
        penetration_warnings.extend(add_penetration_warning_for_intersections(
            &doc.cp,
            &faces,
            &mut result.frame,
            false,
            &intersections,
        ));
        for warning in penetration_warnings {
            if !result.warnings.iter().any(|existing| existing == warning) {
                result.warnings.push(warning.to_string());
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
        // たわみもロックの外で計算する(規約どおり)
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
            // 再生後のライブ操作が同じ解枝から始まるよう、短いロックでwarmだけ更新する。
            lock(&state).store_pose_angles(angles.clone());
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
            contact_detected,
            flat_fold_violations,
        })
    }))
}

/// 円充填のやり直し回数。多いほど良い配置に当たりやすいが時間もかかる。
/// 8回で12本の角でも数百ms以内に収まる(packing.rsの調整値と揃えてある)。
const PACK_STARTS: usize = 8;

/// 提案された折り方の共通部分(作業27 / PRO-009)。
///
/// 折り方を探すのも、通して確かめるのも `crates/ori3-propose` の仕事で、
/// ここは**その結果を画面まで運ぶだけ**である(PRO-009「Tauriホスト内の探索本体0件」)。
///
/// `steps` に入るのは**通して確かめられた手だけ**なので、そのまま作品へ入れられる。
/// 探索が見つけた手数(`planned`)と確かめられた手数(`checked`)を別々に持つのは、
/// 「どこまで確かめられたか」を画面が言えるようにするためである。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalFoldPlanDetails {
    /// 確かめられた折り手順。先頭から順に折る。
    pub steps: Vec<FoldStep>,
    /// その手順を折り込んだ展開図。折る過程で山谷が決まるので、
    /// [`ProposalCandidate::cp`] とは線の種類が違うことがある。
    pub cp: CreasePattern,
    /// 探索が見つけた手の数。
    pub planned: usize,
    /// そのうち、最初から通して確かめられた手の数(`steps.len()` と同じ)。
    pub checked: usize,
}

/// 提案された折り方1つ分。完成まで確認できた手順と、途中までの参考手順を型で分ける。
///
/// JSONでは `status` が判別子になり、共通部分は従来と同じ深さへ平らに並ぶ。
/// `checked_to_finish` という書き換え可能なboolは持たない。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalFoldPlan {
    #[serde(flatten)]
    state: ProposalFoldPlanState,
}

/// JSONへ出す状態の判別子。公開せず、[`ProposalFoldPlan::from_verified`] だけが作る。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ProposalFoldPlanState {
    /// 探索が完成へ到達し、全手順と完成形を改めて確かめた。
    CheckedToFinish {
        #[serde(flatten)]
        details: ProposalFoldPlanDetails,
    },
    /// 打ち切りまでに安全に確認できた参考手順。
    Partial {
        #[serde(flatten)]
        details: ProposalFoldPlanDetails,
    },
}

impl ProposalFoldPlan {
    /// 表示・適用に共通して使う手順と展開図。
    #[must_use]
    pub fn details(&self) -> &ProposalFoldPlanDetails {
        match &self.state {
            ProposalFoldPlanState::CheckedToFinish { details }
            | ProposalFoldPlanState::Partial { details } => details,
        }
    }

    /// 完成まで確認できた型か。保存したboolではなくvariantから一意に決まる。
    #[must_use]
    pub fn checked_to_finish(&self) -> bool {
        matches!(&self.state, ProposalFoldPlanState::CheckedToFinish { .. })
    }

    /// 検証クレートが作った証明型だけを、画面へ返す判別型へ移す。
    ///
    /// 完成の証明後に展開図と手順を組み直せなかった場合も、完成とは名乗らない。
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

/// 折り方を探すときの打ち切り。
///
/// **実測を根拠に決めた**(`scratchpad/propose-27-29-report.md` 段階1)。
/// 提案が作った展開図は折り鶴・やっこさんより折り目が多いので、
/// [`SearchBudget::DEFAULT`](ori3_propose::SearchBudget::DEFAULT) の
/// `max_states = 12` / `branch = 3` のままでは、利用者が
/// 「展開図を作ってもらう」を押してから待つ時間が長くなりすぎる。
///
/// # 2026-08-23に測り直した(**前の表はもう成り立たない**)
///
/// つぶし折り・花弁折りの候補を既定で作るようにし
/// (`crates/ori3-propose/src/enumerate.rs` の `WITH_EXTRA_CANDIDATES`)、
/// あわせて候補4件を[`proposal_generate`]が**同時に**計算するようにした。
/// 前の表は「候補を増やす前・1件ずつ順番に計算・debugビルド」の値で、
/// いまの動きとは条件が3つとも違う。
///
/// ## 測定の条件
///
/// **最適化あり**、`WITH_EXTRA_CANDIDATES = true`、候補4件を同時に計算、
/// 紙150×150mm、種1、`PACK_STARTS = 8`。骨格は根1つ+同じ長さの角。
/// 測るのは**利用者がボタンを押してから候補が全部返るまで**
/// (`pack` → `generate` → 折り方の探索 → 21姿勢の確認 → 手順の組み直し、を候補ぶん)。
/// 各10回の**最大**。
///
/// ## `max_states` / `branch` の決め方(先端12本＝[`ori3_propose::MAX_LEAVES`]の上限で比べた)
///
/// | `max_states` / `branch` | 待ち時間 | 折り方が付いた候補 | 手数合計 |
/// |---|---:|---:|---:|
/// | **2 / 2(採用)** | **13.851秒** | **4件** | **8手** |
/// | 2 / 1 | 12.893秒 | 4件 | 8手 |
/// | 1 / 2 | 3.223秒 | 4件 | **4手** |
/// | 1 / 1 | 2.643秒 | 4件 | **4手** |
///
/// `max_states` を1へ下げると3.2秒まで縮むが、**手数が8手→4手へ半減する**。
/// つぶし折り・花弁折りの候補を作る前の値が**6手**なので、
/// **1 / 2 と 1 / 1 は、候補を増やす前より悪くなる**。
/// `branch` を1へ下げても1秒しか縮まないので、**2 / 2 のままにする**。
///
/// ## 骨格の大きさごとの待ち時間(採用した 2 / 2 で)
///
/// | 骨格 | 待ち時間(最大) | 折り方が付いた候補 | 手数合計 | 探索の止まり方 |
/// |---|---:|---:|---:|---|
/// | 先端 4本 | 0.005秒 | 0件 | 0手 | すべて `GoalReached` |
/// | 先端 6本 | 1.044秒 | 2件 | 3手 | すべて `GoalReached` |
/// | 先端 8本 | 2.162秒 | 1件 | 2手 | `GoalReached` 3 / `StateCap` 1 |
/// | 先端 10本 | 6.367秒 | 3件 | 6手 | すべて `GoalReached` |
/// | **先端 12本** | **13.851秒** | **4件** | **8手** | `StateCap` 3 / `GoalReached` 1 |
///
/// **どの大きさでも10回とも同じ結果**で、watchdog到達は1件も無かった。
///
/// ## `max_millis`(壁時計時間の上限)は**安全弁**であって、打ち切りに使わない
///
/// **前は 6,000ms だった。これは誤りだった。**
/// 先端12本では候補4件のうち2件がこの打ち切りに当たっており、
/// **同じ入力でも機械の混み具合で答えが変わっていた**
/// (実測: 同じ直列の計算で `TimeCap` が2件のときと1件のときがあった)。
/// これは `CLAUDE.md` §10.7.7 が禁じる「解の結果を計算機に依存させる」形である。
/// 速く見えていたのは、**答えを途中で捨てていたから**にすぎない。
///
/// そこで通常結果は**計算量(`max_states` / `branch`)だけで決める**。
/// `max_millis` は別型のwatchdogとして「何かがおかしくて終わらない」ときだけ使う。
///
/// - 実測の最大は **13.851秒**(先端12本、10回の最大)
/// - その **2.2倍**にあたる **30,000ms(30秒)** を安全弁にする
/// - 実測は上限の **46.2%** で、`CLAUDE.md` §10.7.9 の「実測は上限の8割以内」を満たす
///
/// **この値に当たらないことは検査で見張る**
/// (`the_heaviest_proposal_never_hits_the_time_limit`)。将来また重くなったら、
/// 黙って答えが痩せるのではなく検査が落ちる。
///
/// watchdogに到達した場合は途中の最善を返さず、候補全体を専用の内部エラー経路へ
/// 送る。`ProposalFoldPlanState::Partial` は状態数・深さなど決定的な通常停止専用である。
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlanBudget {
    deterministic: SearchBudget,
    watchdog: SearchWatchdog,
}

const PLAN_BUDGET: PlanBudget = PlanBudget {
    deterministic: SearchBudget {
        max_states: 2,
        branch: 2,
        max_depth: SearchBudget::DEFAULT.max_depth,
        rank_scan: SearchBudget::DEFAULT.rank_scan,
        scan: SearchBudget::DEFAULT.scan,
    },
    watchdog: SearchWatchdog { max_millis: 30_000 },
};

/// 確かめ済みの手順から、展開図と手順を組み直すときに見る姿勢の数。
///
/// 折り上がり(`t = 1`)の1点だけを見る。ここへ渡すのは
/// [`PoseScan::DEFAULT`](ori3_propose::PoseScan::DEFAULT)(21点)で
/// **すでに通った手だけ**で、この呼び出しは「同じ手をもう一度選び直して進める」
/// ためのものだからである。21点の部分集合なので、通った手がここで落ちることはない。
const PLAN_REBUILD_SCAN: PoseScan = PoseScan { steps: 0 };

/// 提案された展開図1つ分。`scale` は骨格の長さ1あたりが紙の何割になるか(大きいほど
/// 完成品が大きい)、`violations` は平坦に折りにくい頂点の数(0が理想)。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalCandidate {
    pub cp: CreasePattern,
    pub scale: f64,
    pub violations: usize,
    pub warnings: Vec<String>,
    /// 骨格の先端1本ずつが、この展開図のどの点・どの分子になったかの対応
    /// (作業9 / PRO-007)。候補ごとに配置が違うので候補ごとに持つ。
    /// 先端1本につきちょうど1件入る。
    pub sites: Vec<LeafSite>,
    /// この展開図の折り方(作業27)。折り方が1手も見つからなかったときは `None`。
    pub fold_plan: Option<ProposalFoldPlan>,
}

/// 画面が作ってbackendへ渡す、不透明な提案job ID。
///
/// backendは値の中身を解釈せず、同じIDの二重登録を拒否するためだけに使う。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProposalJobId(String);

impl From<String> for ProposalJobId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ProposalJobId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
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

impl ProposalPhase {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Finished | Self::Cancelled | Self::Failed)
    }
}

/// 1つのjobについて、同じ時点から読み取った進捗と実行状態。
///
/// `cancel_requested` も同じMutexの中に置く。取消しだけを別atomicへ分けると、
/// phaseと取消し状態が別時点の値になり、後続の探索が古い値を読めてしまう。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProposalProgressSnapshot {
    pub job_id: ProposalJobId,
    pub done: usize,
    pub total: usize,
    pub phase: ProposalPhase,
    #[serde(skip)]
    cancel_requested: bool,
}

/// 1つのjobの正本。`done / total / phase / cancel` は常にこのMutexを1回だけlockして扱う。
struct ProposalProgressCell {
    snapshot: Mutex<ProposalProgressSnapshot>,
}

impl ProposalProgressCell {
    fn new(job_id: ProposalJobId) -> Self {
        Self {
            snapshot: Mutex::new(ProposalProgressSnapshot {
                job_id,
                done: 0,
                total: 0,
                phase: ProposalPhase::Queued,
                cancel_requested: false,
            }),
        }
    }

    fn lock_snapshot(&self) -> MutexGuard<'_, ProposalProgressSnapshot> {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 候補総数が決まった時点で生成段階へ進める。cancel済みのjobは戻さない。
    fn start(&self, total: usize) -> Result<(), SearchAbort> {
        let mut snapshot = self.lock_snapshot();
        if snapshot.cancel_requested {
            return Err(SearchAbort::Cancelled);
        }
        snapshot.done = 0;
        snapshot.total = total;
        snapshot.phase = ProposalPhase::Generating;
        Ok(())
    }

    /// 折り方の検証へ入ったことを記録する。複数workerが呼んでも後戻りしない。
    fn begin_verifying(&self) -> Result<(), SearchAbort> {
        let mut snapshot = self.lock_snapshot();
        if snapshot.cancel_requested {
            return Err(SearchAbort::Cancelled);
        }
        if !snapshot.phase.is_terminal() {
            snapshot.phase = ProposalPhase::Verifying;
        }
        Ok(())
    }

    fn check_cancelled(&self) -> Result<(), SearchAbort> {
        if self.lock_snapshot().cancel_requested {
            Err(SearchAbort::Cancelled)
        } else {
            Ok(())
        }
    }

    /// 候補1件ぶんの計算が終わった。全ticketを加算し、超過はFailedとして可視化する。
    fn finish_one(&self) {
        let mut snapshot = self.lock_snapshot();
        let Some(done) = snapshot.done.checked_add(1) else {
            snapshot.phase = ProposalPhase::Failed;
            return;
        };
        snapshot.done = done;
        if snapshot.done > snapshot.total && !snapshot.phase.is_terminal() {
            snapshot.phase = ProposalPhase::Failed;
        }
    }

    fn snapshot(&self) -> ProposalProgressSnapshot {
        self.lock_snapshot().clone()
    }
}

impl SearchCancellation for ProposalProgressCell {
    fn is_cancelled(&self) -> bool {
        self.lock_snapshot().cancel_requested
    }
}

/// 実行中の提案jobだけを保持するTauri managed state。
#[derive(Default)]
pub struct ProposalJobs {
    jobs: Mutex<HashMap<ProposalJobId, Arc<ProposalProgressCell>>>,
}

impl ProposalJobs {
    fn lock_jobs(&self) -> MutexGuard<'_, HashMap<ProposalJobId, Arc<ProposalProgressCell>>> {
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn start(&self, job_id: ProposalJobId) -> Result<ProposalJobLease<'_>, String> {
        let job = Arc::new(ProposalProgressCell::new(job_id.clone()));
        let mut jobs = self.lock_jobs();
        if jobs.contains_key(&job_id) {
            return Err(format!("同じ提案job IDは同時に使えません: {}", job_id.0));
        }
        jobs.insert(job_id.clone(), Arc::clone(&job));
        drop(jobs);
        Ok(ProposalJobLease {
            registry: self,
            job_id,
            job,
            active: true,
        })
    }

    #[must_use]
    pub fn snapshot(&self, job_id: &ProposalJobId) -> Option<ProposalProgressSnapshot> {
        let job = self.lock_jobs().get(job_id).cloned()?;
        Some(job.snapshot())
    }

    fn finish(
        &self,
        job: &Arc<ProposalProgressCell>,
        phase: ProposalPhase,
    ) -> ProposalProgressSnapshot {
        let mut snapshot = job.lock_snapshot();
        if phase == ProposalPhase::Cancelled {
            snapshot.cancel_requested = true;
        }
        if snapshot.cancel_requested {
            snapshot.phase = ProposalPhase::Cancelled;
        } else if !snapshot.phase.is_terminal() {
            snapshot.phase = phase;
        }
        snapshot.clone()
    }

    pub fn cancel(&self, job_id: &ProposalJobId) -> Result<ProposalProgressSnapshot, String> {
        let job = self
            .lock_jobs()
            .get(job_id)
            .cloned()
            .ok_or_else(|| format!("提案jobが見つかりません: {}", job_id.0))?;
        let mut snapshot = job.lock_snapshot();
        match snapshot.phase {
            ProposalPhase::Finished | ProposalPhase::Failed => {
                Err(format!("提案jobはすでに終了しています: {}", job_id.0))
            }
            ProposalPhase::Cancelled => Ok(snapshot.clone()),
            ProposalPhase::Queued | ProposalPhase::Generating | ProposalPhase::Verifying => {
                snapshot.cancel_requested = true;
                snapshot.phase = ProposalPhase::Cancelled;
                Ok(snapshot.clone())
            }
        }
    }

    fn prune(&self, job_id: &ProposalJobId, job: &Arc<ProposalProgressCell>) -> bool {
        let mut jobs = self.lock_jobs();
        let current_is_same = jobs
            .get(job_id)
            .is_some_and(|current| Arc::ptr_eq(current, job));
        if current_is_same {
            jobs.remove(job_id);
        }
        current_is_same
    }

    #[cfg(test)]
    fn registered_count(&self) -> usize {
        self.lock_jobs().len()
    }
}

/// 登録したjobを、正常・Err・cancel・panicのどの出口でも必ず回収する札。
struct ProposalJobLease<'a> {
    registry: &'a ProposalJobs,
    job_id: ProposalJobId,
    job: Arc<ProposalProgressCell>,
    active: bool,
}

impl ProposalJobLease<'_> {
    fn complete(mut self, phase: ProposalPhase) -> ProposalProgressSnapshot {
        let snapshot = self.registry.finish(&self.job, phase);
        self.registry.prune(&self.job_id, &self.job);
        self.active = false;
        snapshot
    }
}

impl Drop for ProposalJobLease<'_> {
    fn drop(&mut self) {
        if self.active {
            self.registry.finish(&self.job, ProposalPhase::Failed);
            self.registry.prune(&self.job_id, &self.job);
        }
    }
}

/// 候補1件ぶんの「終わった」を、**うまくいっても、失敗しても、途中で落ちても**
/// ちょうど1つだけ数えるための札。
///
/// 数を足す行を計算の後ろに置くだけだと、計算が途中で落ちたときにその1件が
/// 数え落とされ、画面の棒が最後まで伸びないまま終わる。
/// 札は巻き戻し(unwind)の途中でも捨てられるので、落ちても必ず数える。
/// 「折り方が見つからなかった」も「終わった」うちに入れる。
struct CandidateTicket<'a>(&'a ProposalProgressCell);

impl Drop for CandidateTicket<'_> {
    fn drop(&mut self) {
        self.0.finish_one();
    }
}

/// 指定した提案jobの、同じ時点の進捗snapshotを返す。
/// 完了後・取消し完了後・未知IDは、registryから回収済みなので`None`になる。
#[tauri::command]
#[must_use]
pub fn proposal_progress(
    jobs: State<'_, ProposalJobs>,
    job_id: ProposalJobId,
) -> Option<ProposalProgressSnapshot> {
    jobs.snapshot(&job_id)
}

/// 同種のjob操作をまとめる入口。1-Bでは取消しだけを持つ。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProposalControl {
    Cancel { job_id: ProposalJobId },
}

#[tauri::command]
pub fn proposal_control(
    jobs: State<'_, ProposalJobs>,
    operation: ProposalControl,
) -> Result<ProposalProgressSnapshot, String> {
    match operation {
        ProposalControl::Cancel { job_id } => jobs.cancel(&job_id),
    }
}

/// 候補全体を返さずに終える理由。探索中断を通常の候補生成失敗と混ぜない。
#[derive(Debug, PartialEq, Eq)]
enum ProposalGenerationError {
    Input(String),
    SearchAborted(SearchAbort),
}

impl std::fmt::Display for ProposalGenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(message) => f.write_str(message),
            Self::SearchAborted(SearchAbort::WatchdogExpired) => f.write_str(
                "提案の探索が見張り時間を超えたため中断しました(途中の候補は返していません)",
            ),
            Self::SearchAborted(SearchAbort::Cancelled) => {
                f.write_str("提案の計算を取り消しました(途中の候補は返していません)")
            }
        }
    }
}

/// worker内の回復可能な生成失敗と、候補全体を捨てる探索中断を区別する。
#[derive(Debug)]
enum CandidateBuildError {
    Generation(String),
    SearchAborted(SearchAbort),
}

/// 展開図1つぶんの折り方を探し、通して確かめて、画面へ運べる形にする(作業27)。
///
/// 手順が1手も残らなかったときは `None` を返す。**折れない手を「折れる」として
/// 返さない**ため、確かめられた手だけを `steps` へ入れる。
///
/// 探索の予算は引数で受け取る。画面からは [`PLAN_BUDGET`] がそのまま渡る。
/// 引数にしてあるのは、**壁時計の打ち切りに当たると答えが機械の速さで変わる**ため、
/// 「同時に計算しても1件ずつでも同じ答えになる」ことを見る検査だけが、
/// 打ち切りの起きない予算を渡せるようにするためである(`CLAUDE.md` §10.7.7)。
fn plan_folds(
    skeleton: &Skeleton,
    packing: &Packing,
    cp: &CreasePattern,
    sites: &[LeafSite],
    paper: &Paper,
    budget: PlanBudget,
    cancellation: &dyn SearchCancellation,
) -> Result<Option<ProposalFoldPlan>, SearchAbort> {
    // 紙の長辺を1.0とした大きさ(`ori3_model::Document::new` と同じ正規化)。
    // 呼び出し側とまったく同じ式なので、受け取る代わりにここで出す。
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
        // どの紙の場所がどの先端になるかは作業9の対応をそのまま渡す。
        // 座標から相手を当てにいく経路は作らない(PRO-007)。
        sites: sites
            .iter()
            .map(|s| TipSite {
                leaf_id: s.circle.leaf_id,
                material: s.vertex.map_or(s.circle.center, |v| v.pos),
            })
            .collect(),
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
    let order: Vec<usize> = outcome.steps.iter().map(|s| s.mv.id).collect();
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
    // 通った手だけをもう一度たどって、展開図と手順を取り出す。
    let mut walk = session.clone();
    for step in &report.steps {
        if cancellation.is_cancelled() {
            return Err(SearchAbort::Cancelled);
        }
        let Some(Ok(mv)) = walk.check_move(step.id, PLAN_REBUILD_SCAN) else {
            break;
        };
        if walk.apply(&mv).is_err() {
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

/// 骨格から展開図の候補を作る(PRO-001/PRO-005、Task 3-4)。
/// 乱数の初期値違いで最大4つの候補を返し、どれを使うかは利用者が選ぶ。
///
/// `with_fold_plan` が真なら、候補ごとに折り方も探して付ける(作業27)。
/// 展開図を作るだけなら1秒もかからないが、折り方を探すのは実測で
/// 候補4件あわせて5〜10秒(debugビルド)かかる。展開図だけを見たい検査が
/// その時間を払わずに済むよう、付けるかどうかを呼び出し側が決める。
/// 画面はいつも真で呼ぶ。
///
/// 設計規約: ロック中に重い計算をしない。この処理は作品の状態を一切見ないので
/// storeのロックそのものを取らない(充填中も他のコマンドが普通に動く)。
/// 1-Bより前からRust内で使われている4引数入口。
/// `store.rs`の受入検査を壊さず、製品IPCとは別のlocal registryで同じ本体を通す。
pub fn proposal_generate(
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
    with_fold_plan: bool,
) -> Result<Vec<ProposalCandidate>, String> {
    let jobs = ProposalJobs::default();
    proposal_generate_managed(
        &jobs,
        ProposalJobId::from("rust-direct"),
        skeleton,
        paper,
        seed,
        with_fold_plan,
    )
    .map(|result| result.candidates)
}

/// job IDを結果にもechoし、別要求の応答を取り違えないようにする。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalJobResult {
    pub job_id: ProposalJobId,
    pub candidates: Vec<ProposalCandidate>,
}

/// Tauri `State`を偽造せず、製品と同じjob lifecycleを検査できる内側。
fn proposal_generate_managed(
    jobs: &ProposalJobs,
    job_id: ProposalJobId,
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
    with_fold_plan: bool,
) -> Result<ProposalJobResult, String> {
    let result_job_id = job_id.clone();
    let candidates = run_proposal_job(jobs, job_id, |progress| {
        generate_candidates(
            &skeleton,
            &paper,
            seed,
            with_fold_plan,
            PLAN_BUDGET,
            progress,
        )
    })?;
    Ok(ProposalJobResult {
        job_id: result_job_id,
        candidates,
    })
}

/// job本体のpanicをleaseより内側で捕捉し、cancelとの競合も同じterminal値へそろえる。
fn run_proposal_job<T>(
    jobs: &ProposalJobs,
    job_id: ProposalJobId,
    body: impl FnOnce(&ProposalProgressCell) -> Result<T, ProposalGenerationError>,
) -> Result<T, String> {
    guard(AssertUnwindSafe(move || {
        let lease = jobs.start(job_id)?;
        let generated = std::panic::catch_unwind(AssertUnwindSafe(|| body(&lease.job)));
        match generated {
            Ok(Ok(value)) => {
                let terminal = lease.complete(ProposalPhase::Finished);
                if terminal.phase == ProposalPhase::Cancelled {
                    Err(ProposalGenerationError::SearchAborted(SearchAbort::Cancelled).to_string())
                } else if terminal.phase == ProposalPhase::Finished {
                    Ok(value)
                } else {
                    Err("提案jobの進捗が不整合になりました".to_string())
                }
            }
            Ok(Err(error)) => {
                let phase =
                    if error == ProposalGenerationError::SearchAborted(SearchAbort::Cancelled) {
                        ProposalPhase::Cancelled
                    } else {
                        ProposalPhase::Failed
                    };
                let terminal = lease.complete(phase);
                if terminal.phase == ProposalPhase::Cancelled {
                    Err(ProposalGenerationError::SearchAborted(SearchAbort::Cancelled).to_string())
                } else {
                    Err(error.to_string())
                }
            }
            Err(payload) => {
                let terminal = lease.complete(ProposalPhase::Failed);
                if terminal.phase == ProposalPhase::Cancelled {
                    Err(ProposalGenerationError::SearchAborted(SearchAbort::Cancelled).to_string())
                } else {
                    std::panic::resume_unwind(payload)
                }
            }
        }
    }))
}

/// 画面から呼ぶmanaged入口。Rust名は既存4引数関数と分けるが、IPC名は従来どおり。
#[tauri::command(async, rename = "proposal_generate")]
pub fn proposal_generate_job(
    jobs: State<'_, ProposalJobs>,
    job_id: ProposalJobId,
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
    with_fold_plan: bool,
) -> Result<ProposalJobResult, String> {
    proposal_generate_managed(&jobs, job_id, skeleton, paper, seed, with_fold_plan)
}

/// 候補を作る本体。**進み具合を書き込む先を引数で受け取る**。
///
/// managed入口は登録済みjobの[`ProposalProgressCell`]を渡す。
/// registryの鍵を計算中に保持しないため、別jobの生成と進捗読取りは同時に進められる。
fn generate_candidates(
    skeleton: &Skeleton,
    paper: &Paper,
    seed: u64,
    with_fold_plan: bool,
    budget: PlanBudget,
    progress: &ProposalProgressCell,
) -> Result<Vec<ProposalCandidate>, ProposalGenerationError> {
    progress
        .check_cancelled()
        .map_err(ProposalGenerationError::SearchAborted)?;
    skeleton
        .validate()
        .map_err(ProposalGenerationError::Input)?;
    let long = paper.width_mm.max(paper.height_mm);
    if !(long > 0.0 && long.is_finite()) {
        return Err(ProposalGenerationError::Input(
            "紙のサイズは正の値にしてください".to_string(),
        ));
    }
    // CPの座標系は「紙の長辺=1.0」正規化(ori3_model::Document::new と同じ)
    let (w, h) = (paper.width_mm / long, paper.height_mm / long);
    let packings = pack(skeleton, w, h, seed, PACK_STARTS);
    progress
        .check_cancelled()
        .map_err(ProposalGenerationError::SearchAborted)?;
    // 候補は互いに独立なので、同時に計算する(理由と実測は `plan_folds` のコメント)。
    // 進み具合は画面が `proposal_progress` で読む。
    progress
        .start(packings.len())
        .map_err(ProposalGenerationError::SearchAborted)?;
    let planned: Vec<Result<ProposalCandidate, CandidateBuildError>> =
        std::thread::scope(|scope| {
            let workers: Vec<_> = packings
                .iter()
                .map(|p| {
                    scope.spawn(move || {
                        // 「終わった」を先に予約しておく。うまくいっても、失敗しても、
                        // 途中で落ちても、この札が捨てられるときにちょうど1つ数える。
                        let _ticket = CandidateTicket(progress);
                        progress
                            .check_cancelled()
                            .map_err(CandidateBuildError::SearchAborted)?;
                        let r =
                            generate(skeleton, p, w, h).map_err(CandidateBuildError::Generation)?;
                        progress
                            .check_cancelled()
                            .map_err(CandidateBuildError::SearchAborted)?;
                        // 折り方が見つからない通常結果はNoneで候補を返す。一方watchdog/cancel
                        // は候補単位の欠落へ変換せず、専用Errのまま全体集約へ渡す。
                        let fold_plan = if with_fold_plan {
                            progress
                                .begin_verifying()
                                .map_err(CandidateBuildError::SearchAborted)?;
                            plan_folds(skeleton, p, &r.cp, &r.sites, paper, budget, progress)
                                .map_err(CandidateBuildError::SearchAborted)?
                        } else {
                            None
                        };
                        Ok(ProposalCandidate {
                            cp: r.cp,
                            scale: p.scale,
                            violations: r.violations,
                            warnings: r.warnings,
                            sites: r.sites,
                            fold_plan,
                        })
                    })
                })
                .collect();
            workers
                .into_iter()
                // 1件でも内部で落ちたら、直列だったときと同じように外側の
                // `guard` まで持ち上げる。握りつぶして「候補が作れなかった」に
                // すり替えない。
                .map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
                })
                .collect()
        });
    progress
        .check_cancelled()
        .map_err(ProposalGenerationError::SearchAborted)?;
    // 返す順番と中身は、直列で回していたときとまったく同じにする。
    let mut out = Vec::new();
    let mut last_err = None;
    let mut search_abort = None;
    for made in planned {
        match made {
            Ok(candidate) => out.push(candidate),
            Err(CandidateBuildError::Generation(error)) => last_err = Some(error),
            Err(CandidateBuildError::SearchAborted(abort)) if search_abort.is_none() => {
                search_abort = Some(abort);
            }
            Err(CandidateBuildError::SearchAborted(_)) => {}
        }
    }
    if let Some(abort) = search_abort {
        return Err(ProposalGenerationError::SearchAborted(abort));
    }
    if out.is_empty() {
        return Err(ProposalGenerationError::Input(last_err.unwrap_or_else(
            || {
                "この骨格を紙の上に配置できませんでした(角を減らすか短くしてみてください)"
                    .to_string()
            },
        )));
    }
    Ok(out)
}

/// 提案の展開図と折り手順を、利用者の1操作としてまとめて入れる(作業28 / PRO-003)。
///
/// 展開図だけを差し替える [`EditOp::ReplaceCreasePattern`] は折り手順を必ず空にする。
/// 折り方が付いた提案では、展開図と手順を**同じ1回**で入れないと、
/// 途中の状態(展開図だけ入って手順が無い)が「元に戻す」の対象になってしまう。
#[tauri::command(async)]
pub fn proposal_apply(
    state: State<'_, Mutex<DocumentStore>>,
    cp: CreasePattern,
    steps: Vec<FoldStep>,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(move || {
        let mut args = Some((cp, steps));
        view_command(&state, || {
            let (cp, steps) = args.take().expect("view_commandは1回だけ呼ぶ");
            lock(&state).apply_proposal(cp, steps)
        })
    }))
}

/// 書き出しの種類(EXP-001/EXP-002、Task 4-3)。
/// 折り図の書き出し(Task 4-4/4-5)はコマンドを増やさずここへ足す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportKind {
    /// 展開図の画像(SVG、実寸mm)
    CpSvg,
    /// 展開図の画像(PNG)
    CpPng,
    /// 折り図(PDF。A4に1ページ6コマ、表紙つき)
    DiagramPdf,
    /// 折り図の画像(SVG。ページごとに別ファイル)
    DiagramSvg,
    /// FOLD 1.2 限定(JSON、単一ファイル)
    FoldJson,
}

/// 書き出しの細かい指定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportOptions {
    /// 補助線(下書きの線)も一緒に書き出すか。
    pub include_aux: bool,
    /// PNGのときの長いほうの辺の点数。
    ///
    /// 0以下の指定も受け取れるよう広めのi64にしてある。u32で受け取ると、たとえば
    /// -5のような値はTauriがJSONを読む段階で弾かれ、英語のエラーが画面に出てしまう。
    /// ここで受け取っておけば [`png_long_side`] が日本語で理由を返せる(設計原則3b)。
    pub png_long_side: i64,
}

/// PNGの点数の指定を確かめて、使える値に直す。
///
/// 無理な指定(0以下・上限超え)はボタンを消さずに日本語で理由を伝える(設計原則3b)。
fn png_long_side(value: i64) -> Result<u32, String> {
    if value <= 0 {
        return Err(format!("画像の大きさは1以上にしてください(指定: {value})"));
    }
    if value > ori3_export::MAX_LONG_SIDE_PX as i64 {
        return Err(format!(
            "画像の大きさは{}までにしてください(指定: {value})",
            ori3_export::MAX_LONG_SIDE_PX
        ));
    }
    Ok(value as u32)
}

/// 展開図を画像ファイルとして保存する(EXP-001 / EXP-002)。
///
/// 設計規約: ロック中に重い計算・I/Oをしない。ロック下ではDocumentの複製だけを行い、
/// 図の組み立てとファイル書き出しはロックを解放してから実行する。
#[tauri::command(async)]
pub fn document_export(
    state: State<'_, Mutex<DocumentStore>>,
    kind: ExportKind,
    path: String,
    options: ExportOptions,
) -> Result<Vec<FoldIssue>, String> {
    guard(AssertUnwindSafe(move || {
        let doc = lock(&state).export_inputs(); // 複製のみ、即ロック解放
        // 先に全ページぶんを作り切る。途中の手順で失敗しても、その時点では
        // まだ1つもファイルを作っていないので中途半端な結果が残らない
        // 全ページを先にメモリへ作り、次に全て一時ファイルへ書く。途中で失敗しても
        // 既存の完成ファイルには触れない。
        let (files, fold_issues) = export_files(&doc, kind, options)?;
        write_export_files(Path::new(&path), &files)?;
        Ok(fold_issues)
    }))
}

/// 書き出し先のファイル名を決める。折り図の画像はページごとに別ファイルになるので、
/// 選ばれた場所を基準に「鶴-01.svg」「鶴-02.svg」…と番号を足して並べる。
fn export_target(path: &Path, suffix: &str) -> std::path::PathBuf {
    if suffix.is_empty() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("折り図");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("svg");
    path.with_file_name(format!("{stem}{suffix}.{ext}"))
}

fn export_temp_path(target: &Path) -> std::path::PathBuf {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("export");
    let id = EXPORT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{name}.{}.{}.export.tmp", std::process::id(), id))
}

/// 全ページを一時ファイルへ出してから完成名へ切り替える。
///
/// 切り替え中に失敗した場合に消すのは、今回初めて作った完成名だけである。
/// 上書き前からあったファイルは、途中の失敗時にも削除しない。
fn write_export_files(path: &Path, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    let targets: Vec<_> = files
        .iter()
        .map(|(suffix, _)| export_target(path, suffix))
        .collect();
    let existed: Vec<_> = targets.iter().map(|target| target.exists()).collect();
    let mut staged = Vec::with_capacity(files.len());

    for ((_, bytes), target) in files.iter().zip(&targets) {
        let temp = export_temp_path(target);
        if let Err(err) = std::fs::write(&temp, bytes) {
            for done in &staged {
                let _ = std::fs::remove_file(done);
            }
            return Err(format!("ファイルに書き出せませんでした: {err}"));
        }
        staged.push(temp);
    }

    let mut created = Vec::new();
    for ((temp, target), already_existed) in staged.iter().zip(&targets).zip(&existed) {
        if let Err(err) = std::fs::rename(temp, target) {
            // まだ切り替えていない一時ファイルだけを片付ける。
            for pending in &staged {
                let _ = std::fs::remove_file(pending);
            }
            // 今回新規に作った完成ファイルは半端な折り図に見えないよう消す。
            // もともとあった完成ファイルは絶対に消さない。
            for done in &created {
                let _ = std::fs::remove_file(done);
            }
            return Err(format!("ファイルに書き出せませんでした: {err}"));
        }
        if !already_existed {
            created.push(target.clone());
        }
    }
    Ok(())
}

/// 指定の種類で書き出す中身を組み立てる(ロックを取らない純粋な処理)。
///
/// 戻り値は(ファイル名の後ろに足す文字, 中身)の並び。1つのファイルで済む種類は
/// 空文字を返し、折り図の画像(EXP-004)だけページ数ぶんの並びになる。
fn export_files(
    doc: &ori3_model::Document,
    kind: ExportKind,
    options: ExportOptions,
) -> Result<ExportBuild, String> {
    let opts = CpSvgOptions {
        include_aux: options.include_aux,
    };
    let mut fold_issues = Vec::new();
    let files = match kind {
        ExportKind::CpSvg => vec![(String::new(), cp_svg(doc, &opts).into_bytes())],
        ExportKind::CpPng => {
            let px = png_long_side(options.png_long_side)?;
            vec![(String::new(), cp_png(doc, &opts, px)?)]
        }
        ExportKind::DiagramPdf => vec![(String::new(), diagram_pdf(doc)?)],
        ExportKind::DiagramSvg => diagram_svg_pages(doc)?
            .into_iter()
            .enumerate()
            .map(|(i, page)| (format!("-{:02}", i + 1), page.into_bytes()))
            .collect(),
        ExportKind::FoldJson => {
            let user_error = || {
                "この作品はFOLD 1.2 限定として書き出せません。作品の内容を確認してください。"
                    .to_string()
            };
            let FoldExport { file, warnings } = document_to_fold(doc).map_err(|_| user_error())?;
            let json = write_fold_1_2(&file).map_err(|_| user_error())?;
            fold_issues = warnings;
            vec![(String::new(), json.into_bytes())]
        }
    };
    Ok((files, fold_issues))
}

#[cfg(test)]
mod tests {
    #[path = "canonical_pose_contract.rs"]
    mod canonical_pose_contract;
    mod movestep_contract;

    use super::{
        CandidateTicket, DocumentStore, FoldAllLayerOrder, PACK_STARTS, PLAN_BUDGET, PlanBudget,
        ProposalCandidate, ProposalFoldPlan, ProposalFoldPlanDetails, ProposalFoldPlanState,
        ProposalGenerationError, ProposalJobId, ProposalJobs, ProposalPhase, ProposalProgressCell,
        attach_replay, display_soft_settings, fold_all_preview_outcome, frame_surface_rank_order,
        generate_candidates, guard, plan_folds, pose_motion_contact_options, pose_overlap_order,
        pose_result_is_finite, proposal_generate, proposal_generate_managed, record_finish_soft,
        recorded_soft_settings, run_proposal_job, stamp_saved_layer_order,
        usable_pose_surface_order,
    };

    #[test]
    fn pose_solve_request_keeps_all_seven_ipc_values() {
        use super::{PoseSolveMode, PoseSolveRequest};

        let request: PoseSolveRequest = serde_json::from_value(serde_json::json!({
            "hard": [{ "hinge": 19, "target_angle_deg": 90.0 }],
            "preferred": [{ "hinge": 17, "target_angle_deg": -90.0 }],
            "soft": null,
            "warmSeed": [{ "hinge": 21, "target_angle_deg": 45.0 }],
            "upTo": 2,
            "t": 0.4,
            "mode": "Canonical"
        }))
        .expect("画面の7値をrequestから復元できる");

        assert_eq!(request.hard.len(), 1);
        assert_eq!(request.hard[0].hinge, 19);
        assert_eq!(request.hard[0].target_angle_deg, 90.0);
        assert_eq!(request.preferred.as_ref().map(Vec::len), Some(1));
        assert!(request.soft.is_none());
        assert_eq!(request.warm_seed.as_ref().map(Vec::len), Some(1));
        assert_eq!(request.up_to, 2);
        // JSON literal 0.4 の復元実測差は0。計算値のexact比較を避け、
        // wire値を取り違えれば十分検出できる1e-12を境界にする。
        assert!((request.t - 0.4).abs() <= 1e-12);
        assert_eq!(request.mode, Some(PoseSolveMode::Canonical));
    }
    use ori3_model::{
        DisplaySettings, Document, Driver, EdgeId, EdgeKind, Face3D, FaceId, FinishSoftSettings,
        FoldStep, Frame3D, Paper, SeqOp, TechniqueKind, VertexId,
    };
    use ori3_propose::{
        CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, SearchAbort,
        SearchControl, SearchStop, SearchWatchdog, Skeleton, SkeletonNode, TipSite, body_on_paper,
        generate, pack, search_to_completion_with_control,
    };
    use ori3_soft::SoftSettings;
    use std::collections::HashMap;
    use std::panic::AssertUnwindSafe;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// 「同時に計算しても1件ずつでも同じ答えになる」ことだけを見る検査のための予算。
    ///
    /// [`PLAN_BUDGET`] との違いは `max_millis` **だけ**で、
    /// 探索する計算量(`max_states = 2` / `branch = 2`)も、深さも、走査の細かさも同じ。
    /// 壁時計の打ち切りに当たると答えが機械の速さで変わってしまうため
    /// (`CLAUDE.md` §10.7.7)、**どんな機械でも当たらない** 3,600,000ms(1時間)にする。
    ///
    /// 根拠(実測 2026-08-24、最適化なし、単独実行3回): この予算を使う検査
    /// (`proposal_candidates_are_the_same_computed_together_or_one_by_one`、先端6本)は
    /// **47.64 / 47.49 / 47.48秒**で終わる。上限は探索1回ごとに効くので、
    /// この検査は探索を8回(同時4件 + 1件ずつ4件)するから、
    /// **探索1回はどんなに長くても47.7秒を超えない**。上限 3,600,000ms の **1.3%以下**である。
    /// CIの計算機が手元より約3.6倍遅い(§10.6)としても約172秒で、上限の **4.8%以下**。
    /// **打ち切りに当たりようがない。**
    const TIME_FREE_PLAN_BUDGET: PlanBudget = PlanBudget {
        watchdog: SearchWatchdog {
            max_millis: 3_600_000,
        },
        ..PLAN_BUDGET
    };

    /// 実行環境のrandom seedを持たない、検査報告用の固定FNV-1a 64bit hash。
    fn contract_hash(text: &str) -> u64 {
        text.as_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    /// 候補を**1件ずつ順番に**計算する参照実装(`proposal_generate` の並列版と突き合わせる用)。
    ///
    /// `proposal_generate` を並列にする前の輪と同じ順序・同じ中身を作る。
    /// あわせて、候補ごとの探索の止まり方(`stop`)も集める。
    fn proposal_generate_one_by_one(
        skeleton: &Skeleton,
        paper: &Paper,
        seed: u64,
        budget: PlanBudget,
    ) -> (Vec<ProposalCandidate>, Vec<SearchStop>) {
        let long = paper.width_mm.max(paper.height_mm);
        let (w, h) = (paper.width_mm / long, paper.height_mm / long);
        let packings = pack(skeleton, w, h, seed, PACK_STARTS);
        let mut out = Vec::new();
        let mut stops = Vec::new();
        let not_cancelled = || false;
        for p in &packings {
            let Ok(r) = generate(skeleton, p, w, h) else {
                continue;
            };
            // `plan_folds` と同じ探索をもう一度して、止まり方だけを取り出す。
            let mut document = Document::new(paper.clone());
            document.cp = r.cp.clone();
            if let Ok(session) = FoldSession::new(&document) {
                let goal = FoldGoal {
                    target: FinishTarget::from_skeleton(skeleton),
                    body: body_on_paper(skeleton, p, w, h),
                    sites: r
                        .sites
                        .iter()
                        .map(|s| TipSite {
                            leaf_id: s.circle.leaf_id,
                            material: s.vertex.map_or(s.circle.center, |v| v.pos),
                        })
                        .collect(),
                };
                let control = SearchControl::new(budget.watchdog, &not_cancelled);
                stops.push(
                    search_to_completion_with_control(
                        &session,
                        &goal,
                        GapWeights::DEFAULT,
                        budget.deterministic,
                        CompletionTolerance::DEFAULT,
                        &control,
                    )
                    .expect("比較検査用watchdogへ到達しない")
                    .stop,
                );
            }
            let fold_plan = plan_folds(skeleton, p, &r.cp, &r.sites, paper, budget, &not_cancelled)
                .expect("比較検査用watchdogへ到達しない");
            out.push(ProposalCandidate {
                cp: r.cp,
                scale: p.scale,
                violations: r.violations,
                warnings: r.warnings,
                sites: r.sites,
                fold_plan,
            });
        }
        (out, stops)
    }

    /// 同時に計算しても、1件ずつ計算したときと**まったく同じ結果**になること。
    ///
    /// # なぜこの検査が要るか
    ///
    /// `proposal_generate` は候補を同時に計算する。候補どうしは独立だが、
    /// 折り方を探す道の途中に**プロセス全体で共有する枠が1つある**
    /// (`crates/ori3-layers/src/replay.rs` の `ReplayEndpointCache`。
    /// `(document, faces)` の完全一致で照合する1枠だけのキャッシュ)。
    /// 共有する物がある以上、「同時にしても結果は同じ」を**言葉ではなく検査で**固定する。
    ///
    /// 骨格は先端6本。`with_fold_plan = true` にしないと折り方を探す道を通らないので、
    /// 真で呼ぶ(この道にだけ共有の枠がある)。
    ///
    /// # なぜ [`PLAN_BUDGET`] をそのまま使わず、この検査専用の予算にするか
    ///
    /// `PLAN_BUDGET.watchdog.max_millis`(30,000ms)は**壁時計**の見張りである。
    /// 現在は到達すると候補全体が専用Errになり、答えを痩せさせない。ただしこの検査は
    /// 並列・直列の正常結果を比べる目的なので、異常停止しない1時間watchdogを使う。
    /// 旧契約では当たった側だけ答えが痩せ、実際に次のflakeが起きた:
    ///
    /// - 実測(2026-08-24): `cargo build --release` を同時に走らせて機械を混ませた状態で
    ///   この検査が**1回落ちた**。そのとき所要時間は **90.22秒**(普段は約50秒)で、
    ///   同時と1件ずつで折り方の手数が食い違った。
    /// - 先端6本は最適化ありで1.044秒、最適化なしはその16.8〜20.5倍(**17〜21秒**)で、
    ///   30秒の打ち切りまでの余裕が9〜13秒しかない。CIの計算機は手元より
    ///   **約3.6倍遅い**(§10.6)ので、そのままではCIで落ちうる。
    ///
    /// そこで**この検査の中だけ** `max_millis` を [`TIME_FREE_PLAN_BUDGET`] の
    /// 3,600,000ms(1時間)にして、**どんな機械でも壁時計の打ち切りが起きない**ようにする。
    /// 決定的な`max_states` と `branch` は `PLAN_BUDGET` と**同じ 2・2 のまま**なので、
    /// 探索する計算量は画面と同じで、**主張の意味は変わらない**。
    /// `PLAN_BUDGET` そのものの値は変更していない。
    #[test]
    fn proposal_candidates_are_the_same_computed_together_or_one_by_one() {
        let skeleton = star(6);
        let progress = ProposalProgressCell::new(ProposalJobId::from("contract-hash"));
        let together =
            generate_candidates(&skeleton, &A4ISH, 1, true, TIME_FREE_PLAN_BUDGET, &progress)
                .expect("候補が返るはず");
        let (one_by_one, stops) =
            proposal_generate_one_by_one(&skeleton, &A4ISH, 1, TIME_FREE_PLAN_BUDGET);
        assert_eq!(
            together.len(),
            one_by_one.len(),
            "候補の数が違う(同時 {} / 1件ずつ {})",
            together.len(),
            one_by_one.len()
        );
        assert_eq!(
            together, one_by_one,
            "同時に計算した結果が、1件ずつ計算した結果と違う"
        );
        let together_json = serde_json::to_string(&together).expect("候補JSONを作れる");
        let one_by_one_json = serde_json::to_string(&one_by_one).expect("候補JSONを作れる");
        assert_eq!(together_json, one_by_one_json, "正規化前の候補JSONが違う");
        let stop_contract = stops
            .iter()
            .map(|stop| stop.contract_tag())
            .collect::<Vec<_>>()
            .join("|");
        let candidate_hash = contract_hash(&together_json);
        let stop_hash = contract_hash(&stop_contract);
        assert_eq!(
            candidate_hash, 0xb540_4e82_2ccd_3603,
            "1-Aで固定した候補JSON契約が変わった"
        );
        assert_eq!(
            stop_hash, 0xea05_a0f8_b887_39bb,
            "1-Aで固定した通常停止理由契約が変わった"
        );
        println!(
            "candidate_json_fnv1a64={:016x} normal_stop_fnv1a64={:016x} stops={stop_contract}",
            candidate_hash, stop_hash,
        );
    }

    /// 進み具合が、候補の数だけ数えられること。
    ///
    /// 画面の「4件中2件め」の表示(`ProposalWizard.tsx` の `ProposalProgressBar`)は
    /// この数字だけを見ている。**終わった件数が総数に届かないと、
    /// 利用者の画面で棒が最後まで伸びないまま計算だけ終わる。**
    ///
    /// 数を書く先は**この検査だけの入れ物**にする。プロセスにひとつしかない
    /// 画面用の数を見ると、同時に走る別の検査の計算がその数を書き換えてしまう
    /// (実測と経緯は [`ProposalProgressCell`] のコメント)。
    /// 画面の道すじがつながっていることは
    /// [`the_screen_reads_the_numbers_of_the_real_calculation`] が別に見る。
    #[test]
    fn proposal_progress_counts_every_candidate() {
        let progress = ProposalProgressCell::new(ProposalJobId::from("candidate-count"));
        let out = generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &progress)
            .expect("候補が返るはず");
        let counted = progress.snapshot();
        assert_eq!(
            counted.total,
            out.len(),
            "総数が候補の数と合っていない: {counted:?} / 候補{}件",
            out.len()
        );
        assert_eq!(
            counted.done, counted.total,
            "終わったのに、終わった件数が総数に届いていない: {counted:?}"
        );
    }

    /// 候補1件の計算が途中で落ちても、その1件は「終わった」に数えられること。
    ///
    /// 数え落とすと、計算はもう終わっているのに画面の棒が最後まで伸びない。
    /// 「折り方が見つからなかった」も「落ちた」も、終わったうちに入れる。
    #[test]
    fn a_candidate_that_blows_up_is_still_counted_as_finished() {
        let progress = ProposalProgressCell::new(ProposalJobId::from("panic-count"));
        progress.start(1).expect("候補計算を開始できる");
        let fell_over = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _ticket = CandidateTicket(&progress);
            panic!("候補の計算が落ちた");
        }));
        assert!(fell_over.is_err(), "前提: 落ちること");
        let snapshot = progress.snapshot();
        assert_eq!((snapshot.done, snapshot.total), (1, 1));
        assert_eq!(snapshot.phase, ProposalPhase::Generating);
    }

    #[test]
    fn candidate_ticket_overflow_is_visible_instead_of_being_clamped() {
        let progress = ProposalProgressCell::new(ProposalJobId::from("ticket-overflow"));
        progress.start(1).expect("候補計算を開始できる");
        drop(CandidateTicket(&progress));
        drop(CandidateTicket(&progress));
        let snapshot = progress.snapshot();
        assert_eq!((snapshot.done, snapshot.total), (2, 1));
        assert_eq!(snapshot.phase, ProposalPhase::Failed);
    }

    /// 画面が読む数が、**本物の計算**が書いた数であること(道すじの確認)。
    ///
    /// managed生成と進捗読取りが、同じjob IDの同じcellへつながっていることを見る。
    #[test]
    fn the_screen_reads_the_numbers_of_the_real_calculation() {
        let jobs = ProposalJobs::default();
        let job_id = ProposalJobId::from("screen-path");
        let lease = jobs.start(job_id.clone()).expect("jobを登録できる");
        let out = generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &lease.job)
            .expect("候補が返るはず");
        let progress = jobs.snapshot(&job_id).expect("実行中jobを読める");
        assert_eq!(
            progress.total,
            out.len(),
            "画面が読む総数が、計算した候補の数と合っていない: {progress:?} / 候補{}件",
            out.len()
        );
        assert_eq!(
            progress.done, progress.total,
            "画面が読む数が最後まで伸びていない: {progress:?}"
        );
        let terminal = lease.complete(ProposalPhase::Finished);
        assert_eq!(terminal.phase, ProposalPhase::Finished);
        assert_eq!(jobs.registered_count(), 0, "完了jobが回収されていない");
    }

    /// 計算を始めた時点で、前回の数字が残っていないこと。
    ///
    /// 残っていると、画面が一瞬「4/4 件」と出してから0へ戻る。
    /// 骨格が不正で早く止まる場合でも0へ戻っていることを、同じ検査で見る。
    #[test]
    fn proposal_progress_is_cleared_before_the_next_calculation() {
        let jobs = ProposalJobs::default();
        let job_id = ProposalJobId::from("reused-after-prune");
        let first = jobs.start(job_id.clone()).expect("1回目を登録できる");
        generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &first.job)
            .expect("候補が返るはず");
        assert!(
            first.job.snapshot().done > 0,
            "前提: 1回目で数字が入っていること"
        );
        first.complete(ProposalPhase::Finished);
        assert_eq!(jobs.registered_count(), 0);

        let second = jobs
            .start(job_id.clone())
            .expect("回収後は同じIDを再利用できる");
        let cleared = jobs.snapshot(&job_id).expect("2回目を読める");
        assert_eq!(
            (cleared.done, cleared.total),
            (0, 0),
            "次の計算の前に数字が0へ戻っていない: {cleared:?}"
        );
        assert_eq!(cleared.phase, ProposalPhase::Queued);
        second.complete(ProposalPhase::Failed);
        assert_eq!(jobs.registered_count(), 0);
    }

    /// 同時に進むA/Bを100組、決定的なbarrierで交錯させてもsnapshotが混ざらない。
    #[test]
    fn two_proposal_jobs_keep_independent_snapshots_one_hundred_times() {
        let mut swaps = 0usize;
        let mut over_total = 0usize;
        let mut representative = None;

        for round in 0..100 {
            let jobs = ProposalJobs::default();
            let a_id = ProposalJobId::from(format!("job-a-{round}"));
            let b_id = ProposalJobId::from(format!("job-b-{round}"));
            let ready = std::sync::Barrier::new(3);
            let release = std::sync::Barrier::new(3);

            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let lease = jobs.start(a_id.clone()).expect("job Aを登録できる");
                    lease.job.start(2).expect("job Aを開始できる");
                    drop(CandidateTicket(&lease.job));
                    ready.wait();
                    release.wait();
                    drop(CandidateTicket(&lease.job));
                    let terminal = lease.complete(ProposalPhase::Finished);
                    assert_eq!((terminal.done, terminal.total), (2, 2));
                    assert_eq!(terminal.phase, ProposalPhase::Finished);
                });
                scope.spawn(|| {
                    let lease = jobs.start(b_id.clone()).expect("job Bを登録できる");
                    lease.job.start(3).expect("job Bを開始できる");
                    drop(CandidateTicket(&lease.job));
                    drop(CandidateTicket(&lease.job));
                    ready.wait();
                    release.wait();
                    drop(CandidateTicket(&lease.job));
                    let terminal = lease.complete(ProposalPhase::Finished);
                    assert_eq!((terminal.done, terminal.total), (3, 3));
                    assert_eq!(terminal.phase, ProposalPhase::Finished);
                });

                ready.wait();
                let a = jobs.snapshot(&a_id).expect("job Aだけを読める");
                let b = jobs.snapshot(&b_id).expect("job Bだけを読める");
                if a.job_id != a_id || (a.done, a.total) != (1, 2) {
                    swaps += 1;
                }
                if b.job_id != b_id || (b.done, b.total) != (2, 3) {
                    swaps += 1;
                }
                over_total += usize::from(a.done > a.total) + usize::from(b.done > b.total);
                assert_eq!(a.phase, ProposalPhase::Generating);
                assert_eq!(b.phase, ProposalPhase::Generating);
                if round == 0 {
                    representative = Some((a, b));
                }
                release.wait();
            });

            assert_eq!(jobs.registered_count(), 0, "round {round}: jobが残った");
        }

        let (a, b) = representative.expect("代表snapshotがある");
        println!(
            "job A={}/{}/{:?}, job B={}/{}/{:?}, swaps={swaps}, over_total={over_total}",
            a.done, a.total, a.phase, b.done, b.total, b.phase
        );
        assert_eq!(swaps, 0, "A/Bの進捗が入れ替わった");
        assert_eq!(over_total, 0, "doneがtotalを超えた");
    }

    /// success・通常Err・panicの各100候補でticketを必ず完了し、jobも回収する。
    #[test]
    fn candidate_tickets_and_job_leases_close_every_path_one_hundred_times() {
        let jobs = ProposalJobs::default();
        let mut success_done = 0usize;
        let mut error_done = 0usize;
        let mut panic_done = 0usize;

        for round in 0..100 {
            let success_id = ProposalJobId::from(format!("ticket-success-{round}"));
            let success = jobs.start(success_id).expect("success jobを登録できる");
            success.job.start(1).expect("success jobを開始できる");
            let made: Result<(), &str> = {
                let _ticket = CandidateTicket(&success.job);
                Ok(())
            };
            assert!(made.is_ok());
            let terminal = success.complete(ProposalPhase::Finished);
            success_done += usize::from(terminal.done == terminal.total);

            let error_id = ProposalJobId::from(format!("ticket-error-{round}"));
            let error = jobs.start(error_id).expect("error jobを登録できる");
            error.job.start(1).expect("error jobを開始できる");
            let made: Result<(), &str> = {
                let _ticket = CandidateTicket(&error.job);
                Err("注入した通常Err")
            };
            assert!(made.is_err());
            let terminal = error.complete(ProposalPhase::Failed);
            error_done += usize::from(terminal.done == terminal.total);

            let panic_id = ProposalJobId::from(format!("ticket-panic-{round}"));
            let mut panic_job = None;
            let fell_over = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let lease = jobs.start(panic_id).expect("panic jobを登録できる");
                panic_job = Some(Arc::clone(&lease.job));
                lease.job.start(1).expect("panic jobを開始できる");
                let _ticket = CandidateTicket(&lease.job);
                panic!("注入したcandidate panic");
            }));
            assert!(fell_over.is_err());
            let terminal = panic_job.expect("panic後もsnapshotを検査できる").snapshot();
            panic_done += usize::from(terminal.done == terminal.total);
            assert_eq!(terminal.phase, ProposalPhase::Failed);
            assert_eq!(jobs.registered_count(), 0, "round {round}: jobが残った");
        }

        println!(
            "ticket done==total: success={success_done}/100, error={error_done}/100, panic={panic_done}/100"
        );
        assert_eq!((success_done, error_done, panic_done), (100, 100, 100));
        assert_eq!(jobs.registered_count(), 0);
    }

    /// cancelとcandidate panicが競合しても、registry・phase・返却理由をCancelledへそろえる。
    #[test]
    fn cancel_wins_over_candidate_panic_one_hundred_times() {
        let jobs = ProposalJobs::default();
        let expected = ProposalGenerationError::SearchAborted(SearchAbort::Cancelled).to_string();
        for round in 0..100 {
            let job_id = ProposalJobId::from(format!("cancel-panic-{round}"));
            let cancel_id = job_id.clone();
            let result: Result<(), String> = run_proposal_job(&jobs, job_id, |progress| {
                progress.start(1).expect("candidateを開始できる");
                jobs.cancel(&cancel_id).expect("panic前にcancelできる");
                let _ticket = CandidateTicket(progress);
                panic!("cancel後に注入したcandidate panic");
            });
            assert_eq!(
                result,
                Err(expected.clone()),
                "round {round}: 返却理由が不一致"
            );
            assert_eq!(jobs.registered_count(), 0, "round {round}: jobが残った");
        }
    }

    /// cancelは対象jobだけへ入り、全ticketのDrop後に100回ともregistryから回収される。
    #[test]
    fn cancelled_proposal_jobs_are_isolated_and_pruned_one_hundred_times() {
        let jobs = ProposalJobs::default();
        for round in 0..100 {
            let a_id = ProposalJobId::from(format!("cancel-a-{round}"));
            let b_id = ProposalJobId::from(format!("cancel-b-{round}"));
            let a = jobs.start(a_id.clone()).expect("job Aを登録できる");
            let b = jobs.start(b_id.clone()).expect("job Bを登録できる");
            a.job.start(1).expect("job Aを開始できる");
            b.job.start(1).expect("job Bを開始できる");
            let a_ticket = CandidateTicket(&a.job);
            let b_ticket = CandidateTicket(&b.job);

            let cancelled = jobs.cancel(&a_id).expect("job Aを取り消せる");
            assert_eq!(cancelled.phase, ProposalPhase::Cancelled);
            assert_eq!(a.job.check_cancelled(), Err(SearchAbort::Cancelled));
            assert_eq!(b.job.check_cancelled(), Ok(()), "cancelがjob Bへ混ざった");
            assert_eq!(
                jobs.snapshot(&b_id).expect("job Bを読める").phase,
                ProposalPhase::Generating
            );

            drop(a_ticket);
            drop(b_ticket);
            let a_terminal = a.complete(ProposalPhase::Finished);
            let b_terminal = b.complete(ProposalPhase::Finished);
            assert_eq!(a_terminal.phase, ProposalPhase::Cancelled);
            assert_eq!((a_terminal.done, a_terminal.total), (1, 1));
            assert_eq!(b_terminal.phase, ProposalPhase::Finished);
            assert_eq!((b_terminal.done, b_terminal.total), (1, 1));
            assert_eq!(
                jobs.registered_count(),
                0,
                "round {round}: cancel後にjobが残った"
            );
        }
    }

    /// duplicate拒否とArc identity付きpruneで、古いleaseが再利用IDを消さない。
    #[test]
    fn duplicate_and_stale_job_ids_cannot_replace_or_prune_a_live_job() {
        let jobs = ProposalJobs::default();
        for round in 0..100 {
            let job_id = ProposalJobId::from(format!("reused-id-{round}"));
            let old = jobs.start(job_id.clone()).expect("最初のjobを登録できる");
            let old_cell = Arc::clone(&old.job);
            assert!(
                jobs.start(job_id.clone()).is_err(),
                "duplicate IDを受け入れた"
            );
            old.complete(ProposalPhase::Failed);

            let current = jobs.start(job_id.clone()).expect("回収後は同じIDを使える");
            assert!(
                !jobs.prune(&job_id, &old_cell),
                "古いjobが再利用後のjobを消した"
            );
            assert!(jobs.snapshot(&job_id).is_some(), "現在のjobが消えた");
            current.complete(ProposalPhase::Failed);
            assert_eq!(jobs.registered_count(), 0);
        }
    }

    #[test]
    fn proposal_job_wire_contains_one_snapshot_and_echoes_the_id() {
        let jobs = ProposalJobs::default();
        let job_id = ProposalJobId::from("wire-job");
        let lease = jobs.start(job_id.clone()).expect("jobを登録できる");
        lease.job.start(3).expect("jobを開始できる");
        drop(CandidateTicket(&lease.job));
        let snapshot = jobs.snapshot(&job_id).expect("snapshotを読める");
        let json = serde_json::to_value(&snapshot).expect("snapshotをJSONへ運べる");
        assert_eq!(json["job_id"], "wire-job");
        assert_eq!(json["done"], 1);
        assert_eq!(json["total"], 3);
        assert_eq!(json["phase"], "Generating");
        assert!(json.get("cancel_requested").is_none());
        assert_eq!(json.as_object().expect("object").len(), 4);

        let control = serde_json::to_value(super::ProposalControl::Cancel {
            job_id: job_id.clone(),
        })
        .expect("controlをJSONへ運べる");
        assert_eq!(
            control,
            serde_json::json!({"type": "Cancel", "job_id": "wire-job"})
        );
        lease.complete(ProposalPhase::Failed);
        assert_eq!(jobs.registered_count(), 0);

        let result = proposal_generate_managed(&jobs, job_id.clone(), star(2), A4ISH, 7, false)
            .expect("managed入口が候補を返す");
        assert_eq!(result.job_id, job_id);
        assert!(!result.candidates.is_empty());
        let result_json = serde_json::to_value(&result).expect("job結果をJSONへ運べる");
        assert_eq!(result_json["job_id"], "wire-job");
        assert!(result_json["candidates"].is_array());
        assert_eq!(result_json.as_object().expect("object").len(), 2);
        assert_eq!(jobs.registered_count(), 0, "結果返却時にjobが残った");

        let only_root = Skeleton {
            nodes: vec![SkeletonNode::new(0, None, 0.0)],
        };
        assert!(
            proposal_generate_managed(
                &jobs,
                ProposalJobId::from("wire-invalid"),
                only_root,
                A4ISH,
                1,
                false,
            )
            .is_err()
        );
        assert_eq!(jobs.registered_count(), 0, "入力Err後にjobが残った");
    }

    /// 候補ごとの探索の止まり方だけを集める(21姿勢の確認と手順の組み直しはしない)。
    ///
    /// 時間の打ち切りに当たるかどうかは**探索の中で決まる**ので、
    /// 見張るのにその先まで走らせる必要はない。恒久の検査を軽くするために分けてある。
    fn proposal_stops_together(
        skeleton: &Skeleton,
        paper: &Paper,
        seed: u64,
        budget: PlanBudget,
    ) -> Vec<Result<SearchStop, SearchAbort>> {
        let long = paper.width_mm.max(paper.height_mm);
        let (w, h) = (paper.width_mm / long, paper.height_mm / long);
        let packings = pack(skeleton, w, h, seed, PACK_STARTS);
        let not_cancelled = || false;
        std::thread::scope(|scope| {
            let workers: Vec<_> = packings
                .iter()
                .map(|p| {
                    scope.spawn(move || {
                        let r = generate(skeleton, p, w, h).ok()?;
                        let mut document = Document::new(paper.clone());
                        document.cp = r.cp;
                        let session = FoldSession::new(&document).ok()?;
                        let goal = FoldGoal {
                            target: FinishTarget::from_skeleton(skeleton),
                            body: body_on_paper(skeleton, p, w, h),
                            sites: r
                                .sites
                                .iter()
                                .map(|site| TipSite {
                                    leaf_id: site.circle.leaf_id,
                                    material: site
                                        .vertex
                                        .map_or(site.circle.center, |vertex| vertex.pos),
                                })
                                .collect(),
                        };
                        let control = SearchControl::new(budget.watchdog, &not_cancelled);
                        Some(
                            search_to_completion_with_control(
                                &session,
                                &goal,
                                GapWeights::DEFAULT,
                                budget.deterministic,
                                CompletionTolerance::DEFAULT,
                                &control,
                            )
                            .map(|outcome| outcome.stop),
                        )
                    })
                })
                .collect();
            workers
                .into_iter()
                .filter_map(|worker| worker.join().expect("計算の途中で落ちた"))
                .collect()
        })
    }

    /// **いちばん重い骨格でも、watchdogに当たらないこと。**
    ///
    /// # なぜこの検査が要るか
    ///
    /// [`PLAN_BUDGET`] のwatchdogは**壁時計**なので、正常完了できるprofileを
    /// release検査で固定する。到達時は途中候補を返さず、専用Errになる。
    /// 2026-08-23より前は 6,000ms で切っており、先端12本では候補4件のうち
    /// **2件が実際に当たっていた**。同じ計算をもう一度しただけで当たる件数が
    /// 2件→1件と変わることも実測しており、`CLAUDE.md` §10.7.7 が禁じる
    /// 「解の結果を計算機に依存させる」形になっていた。
    ///
    /// いまは通常結果を**計算量だけで決め**、時間の上限は安全弁として残している。
    /// **その安全弁に当たっていないこと**を、ここで見張る。
    /// 将来また重くなったら、黙って答えが痩せるのではなく**この検査が落ちる**。
    ///
    /// 骨格は先端12本([`ori3_propose::MAX_LEAVES`] の上限＝いちばん重い)。
    ///
    /// ## この主張は最適化ありでしか成り立たない(2026-08-24追記)
    ///
    /// watchdog(`PLAN_BUDGET.watchdog.max_millis`)は壁時計であり、
    /// 最適化なしは最適化ありより16.8〜20.5倍遅い(`store.rs` の
    /// `checked_head_tail_four_legs_proposal_is_consumed_and_one_undo_restores_the_work`
    /// で実測済み)。実際に測ると、最適化なし(`cargo test -p desktop --lib`)では
    /// 変更前の契約では先端12本の4候補**すべて**が`TimeCap`に当たった
    /// (`[TimeCap, TimeCap, TimeCap, TimeCap]`、2026-08-24実測)。新契約なら
    /// 4件とも途中候補ではなく`SearchAbort::WatchdogExpired`になる。
    /// 実機(最適化なしビルド)でも、先端12本の「展開図を作ってもらう」で
    /// 4候補中3〜4件が「折り方はまだありません」のまま返り、
    /// 待ち時間は約30〜38秒だった(最適化ありの実測は0件/10回・最大13.851秒)。
    ///
    /// したがって、この検査は`CLAUDE.md` §10.6の`#21`として
    /// **最適化ありの`performance`ジョブでのみ走らせ**、
    /// 最適化なしの通常実行(`cargo test --workspace`(#1)・`scripts/check.ps1`・
    /// `scripts/hooks/pre-commit`・CIの`checks`ジョブ)からは`--skip`で外している。
    /// 検査は消していない。`PLAN_BUDGET`の値も変更していない。
    #[test]
    fn the_heaviest_proposal_never_hits_the_time_limit() {
        let results = proposal_stops_together(&star(12), &A4ISH, 1, PLAN_BUDGET);
        assert!(!results.is_empty(), "候補が1件も作られていない");
        let hit: Vec<_> = results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .collect();
        assert!(
            hit.is_empty(),
            "watchdog/cancelに当たった候補が {}件ある。途中候補は返していない: {results:?}",
            hit.len()
        );
    }

    /// 根1つ+`leaves`本の角(すべて同じ長さ・太さ)の骨格。
    fn star(leaves: u32) -> Skeleton {
        let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
        nodes.extend((1..=leaves).map(|i| SkeletonNode::new(i, Some(0), 1.0)));
        Skeleton { nodes }
    }

    const A4ISH: Paper = Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    };

    #[test]
    fn existing_display_flags_map_to_detection_and_explicit_shape_correction() {
        let defaults = DisplaySettings::default();
        assert_eq!(
            pose_motion_contact_options(
                defaults.overlap_prevention_enabled,
                defaults.penetration_prevention_enabled,
            ),
            ori3_rigid::MotionContactOptions {
                detect: true,
                prevent: false,
            }
        );
        assert_eq!(
            pose_motion_contact_options(true, false),
            ori3_rigid::MotionContactOptions {
                detect: false,
                prevent: true,
            }
        );
    }

    fn finish_step(
        id: u32,
        kind: TechniqueKind,
        finish_soft: Option<FinishSoftSettings>,
    ) -> FoldStep {
        FoldStep {
            id,
            kind,
            drivers: Vec::new(),
            layer_order: None,
            alignment: None,
            finish_soft,
            note: String::new(),
        }
    }

    #[test]
    fn pose_push_records_only_the_current_three_finish_values() {
        let display = DisplaySettings {
            soft_enabled: true,
            soft_stiffness: 0.37,
            soft_pressure: 0.64,
            ..DisplaySettings::default()
        };
        let expected = FinishSoftSettings::from(&display);
        let mut push = SeqOp::PushStep {
            step: finish_step(1, TechniqueKind::Pose, None),
        };
        record_finish_soft(&mut push, &display);
        let SeqOp::PushStep { step } = push else {
            unreachable!()
        };
        assert_eq!(step.finish_soft, Some(expected));

        let original = FinishSoftSettings {
            enabled: false,
            stiffness: 0.12,
            pressure: 0.23,
        };
        let mut already_recorded = SeqOp::PushStep {
            step: finish_step(2, TechniqueKind::Pose, Some(original)),
        };
        record_finish_soft(&mut already_recorded, &display);
        let SeqOp::PushStep { step } = already_recorded else {
            unreachable!()
        };
        assert_eq!(step.finish_soft, Some(original), "記録済み値を上書きしない");

        let mut simple = SeqOp::PushStep {
            step: finish_step(3, TechniqueKind::Simple, None),
        };
        record_finish_soft(&mut simple, &display);
        let SeqOp::PushStep { step } = simple else {
            unreachable!()
        };
        assert_eq!(step.finish_soft, None, "通常手順には記録しない");

        let mut insert = SeqOp::InsertStep {
            index: 0,
            step: finish_step(4, TechniqueKind::Pose, None),
        };
        record_finish_soft(&mut insert, &display);
        let SeqOp::InsertStep { step, .. } = insert else {
            unreachable!()
        };
        assert_eq!(step.finish_soft, None, "並べ替え時に現在値を注入しない");

        let mut update = SeqOp::UpdateStep {
            step: finish_step(5, TechniqueKind::Pose, None),
        };
        record_finish_soft(&mut update, &display);
        let SeqOp::UpdateStep { step } = update else {
            unreachable!()
        };
        assert_eq!(step.finish_soft, None, "注釈更新時に現在値を注入しない");
    }

    #[test]
    fn replay_positions_apply_a_a_b_c_finish_values_and_keep_solver_controls() {
        let a = FinishSoftSettings {
            enabled: true,
            stiffness: 0.17,
            pressure: 0.08,
        };
        let b = FinishSoftSettings {
            enabled: false,
            stiffness: 0.52,
            pressure: 0.41,
        };
        let c = FinishSoftSettings {
            enabled: true,
            stiffness: 0.88,
            pressure: 0.76,
        };
        let mut document = Document::new(A4ISH);
        document.sequence = vec![
            finish_step(1, TechniqueKind::Pose, Some(a)),
            finish_step(2, TechniqueKind::Simple, None),
            finish_step(3, TechniqueKind::Pose, Some(b)),
            finish_step(4, TechniqueKind::Pose, Some(c)),
        ];
        let live = SoftSettings {
            enabled: c.enabled,
            subdivision: 3,
            stiffness: c.stiffness,
            pressure: c.pressure,
            iterations: 37,
        };

        for (up_to, expected) in [(1, a), (2, a), (3, b), (4, c)] {
            let actual = display_soft_settings(&document, up_to, 1.0, Some(live))
                .expect("各位置のたわみ設定");
            assert_eq!(actual.enabled, expected.enabled, "位置{up_to}");
            assert_eq!(actual.stiffness, expected.stiffness, "位置{up_to}");
            assert_eq!(actual.pressure, expected.pressure, "位置{up_to}");
            assert_eq!(actual.subdivision, 3, "細分数は保存値にしない");
            assert_eq!(actual.iterations, 37, "反復数は保存値にしない");
        }

        for t in [0.0, 0.5] {
            let actual = recorded_soft_settings(&document, 3, t, Some(live)).unwrap();
            assert_eq!(actual.stiffness, a.stiffness, "Pose Bの完了前 t={t}");
            assert_eq!(actual.pressure, a.pressure, "Pose Bの完了前 t={t}");
        }
        let completed = recorded_soft_settings(&document, 3, 1.0, Some(live)).unwrap();
        assert_eq!(completed.stiffness, b.stiffness);
        let nonfinite = recorded_soft_settings(&document, 3, f64::NAN, Some(live)).unwrap();
        assert_eq!(nonfinite.stiffness, b.stiffness, "非finiteのtは完了扱い");

        let draft = SoftSettings {
            enabled: false,
            stiffness: 0.29,
            pressure: 0.31,
            ..live
        };
        assert_eq!(
            display_soft_settings(&document, 4, 1.0, Some(draft)),
            Some(draft),
            "最新終点だけは次の仕上げを見ながら調整できる"
        );
        let stored = recorded_soft_settings(&document, usize::MAX, 1.0, Some(draft)).unwrap();
        assert_eq!(stored.stiffness, c.stiffness, "保存値自体は最終位置もC");
        assert_eq!(stored.pressure, c.pressure, "保存値自体は最終位置もC");
    }

    #[test]
    fn legacy_soft_uses_the_existing_live_value_and_new_history_starts_disabled() {
        let live = SoftSettings {
            enabled: true,
            subdivision: 4,
            stiffness: 0.63,
            pressure: 0.27,
            iterations: 19,
        };
        let mut legacy = Document::new(A4ISH);
        legacy.sequence = vec![finish_step(1, TechniqueKind::Pose, None)];
        assert_eq!(
            recorded_soft_settings(&legacy, 0, 1.0, Some(live)),
            Some(live),
            "旧作品は全位置で従来のDisplay由来値を使う"
        );
        assert_eq!(
            recorded_soft_settings(&legacy, 1, 1.0, None),
            None,
            "旧作品のDisplayがオフなら勝手に有効化しない"
        );

        let mut recorded = legacy;
        recorded.sequence.push(finish_step(
            2,
            TechniqueKind::Pose,
            Some(FinishSoftSettings::default()),
        ));
        let before_first = recorded_soft_settings(&recorded, 1, 1.0, Some(live)).unwrap();
        assert!(!before_first.enabled, "未来の有効値を過去へ漏らさない");
        assert_eq!(before_first.stiffness, 0.5);
        assert_eq!(before_first.pressure, 0.0);
        assert_eq!(before_first.subdivision, 4);
        assert_eq!(before_first.iterations, 19);
    }

    fn collect_ori3_fixtures(directory: &Path, paths: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("fixtureディレクトリ {}: {error}", directory.display()))
        {
            let entry = entry.expect("fixture項目を読む");
            let path = entry.path();
            if path.is_dir() {
                collect_ori3_fixtures(&path, paths);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("ori3") {
                paths.push(path);
            }
        }
    }

    #[test]
    fn every_crate_ori3_fixture_loads_through_the_product_reader() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let crates = workspace.join("crates");
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&crates).expect("workspaceのcratesを読む") {
            let crate_path = entry.expect("crate項目を読む").path();
            let fixtures = crate_path.join("tests/fixtures");
            if fixtures.is_dir() {
                collect_ori3_fixtures(&fixtures, &mut paths);
            }
        }
        paths.sort();
        assert!(!paths.is_empty(), ".ori3 fixtureが1件も見つからない");

        let mut legacy = 0usize;
        for path in &paths {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let saved = crate::store::parse_document(&text)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            if saved
                .document
                .sequence
                .iter()
                .all(|step| step.finish_soft.is_none())
            {
                legacy += 1;
            }
        }
        println!(
            "読み込んだ.ori3 fixture: {}件（旧形式: {legacy}件）",
            paths.len()
        );
        assert!(legacy >= 1, "たわみ欄の無い旧fixtureを少なくとも1件読む");
    }

    #[test]
    fn proposal_generate_returns_candidates() {
        let out = proposal_generate(star(4), A4ISH, 7, false).expect("候補が返るはず");
        assert!(!out.is_empty() && out.len() <= 4, "件数={}", out.len());
        for c in &out {
            assert!(c.scale > 0.0, "scale={}", c.scale);
            // 輪郭4辺だけ、ということはない(折り線が引かれている)
            assert!(c.cp.edges.len() > 4, "辺数={}", c.cp.edges.len());
        }
    }

    /// 合格条件4: 画面へ渡る候補に、先端と展開図の対応が欠けずに載っていること。
    /// 先端1〜12本の12通りで、どの候補も先端1本につきちょうど1件を持ち、
    /// 材料点の欠損が0件であることを見る(作業9 / PRO-007)。
    #[test]
    fn proposal_candidates_carry_one_site_per_limb_without_gaps() {
        use std::collections::BTreeSet;
        let mut checked_shapes = 0usize;
        let mut checked_candidates = 0usize;
        for leaves in 1..=12u32 {
            let skeleton = star(leaves);
            let expected: BTreeSet<u32> = skeleton.leaves().into_iter().collect();
            let out = proposal_generate(skeleton, A4ISH, 2026, false).expect("候補が返るはず");
            assert!(!out.is_empty(), "先端{leaves}本で候補が0件");
            for c in &out {
                let got: BTreeSet<u32> = c.sites.iter().map(|s| s.circle.leaf_id).collect();
                assert_eq!(
                    c.sites.len(),
                    leaves as usize,
                    "先端{leaves}本で対応の件数が違う"
                );
                assert_eq!(got, expected, "先端{leaves}本で対応する先端の顔ぶれが違う");
                for site in &c.sites {
                    let v = site
                        .vertex
                        .unwrap_or_else(|| panic!("先端{leaves}本: 材料点が欠けている"));
                    assert!(
                        c.cp.vertices.iter().any(|x| x.id == v.id),
                        "材料点{}がこの展開図に無い",
                        v.id
                    );
                    assert!(!site.molecules.is_empty(), "囲む分子が0個");
                }
                checked_candidates += 1;
            }
            checked_shapes += 1;
        }
        assert_eq!(checked_shapes, 12, "12通りすべてを見ていない");
        // 実測: 先端1〜12本の12通りで候補はのべ45件(先端1本のときだけ候補1件、
        // 残り11通りは上限の4件)。下限12件は「1通りにつき最低1候補」の意味。
        assert!(
            checked_candidates >= 12,
            "候補が{checked_candidates}件しかない"
        );
    }

    #[test]
    fn proposal_generate_is_deterministic() {
        let a = proposal_generate(star(3), A4ISH, 42, false).unwrap();
        let b = proposal_generate(star(3), A4ISH, 42, false).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn proposal_generate_rejects_broken_skeleton() {
        // 角が1本もない(根だけ)骨格は骨格側の検査で日本語のErrになる
        let only_root = Skeleton {
            nodes: vec![SkeletonNode::new(0, None, 0.0)],
        };
        let err = proposal_generate(only_root, A4ISH, 1, false).unwrap_err();
        assert!(err.contains("角"), "err={err}");

        // 紙のサイズが0以下でもErr(パニックにしない)
        let bad_paper = Paper {
            width_mm: 0.0,
            height_mm: 0.0,
        };
        assert!(proposal_generate(star(2), bad_paper, 1, false).is_err());
    }

    #[test]
    fn proposal_fold_plan_wire_has_two_tagged_states_and_null() {
        let details = ProposalFoldPlanDetails {
            steps: vec![finish_step(1, TechniqueKind::Simple, None)],
            cp: Document::new(A4ISH).cp,
            planned: 1,
            checked: 1,
        };
        let checked = ProposalFoldPlan {
            state: ProposalFoldPlanState::CheckedToFinish {
                details: details.clone(),
            },
        };
        let partial = ProposalFoldPlan {
            state: ProposalFoldPlanState::Partial { details },
        };

        for (plan, status) in [(checked, "checked_to_finish"), (partial, "partial")] {
            let json = serde_json::to_value(plan).expect("折り方をJSONへ運べない");
            assert_eq!(json["status"], status);
            assert!(json.get("steps").is_some());
            assert!(json.get("cp").is_some());
            assert!(json.get("checked_to_finish").is_none());
        }
        assert_eq!(
            serde_json::to_value(Option::<ProposalFoldPlan>::None)
                .expect("折り方なしをJSONへ運べない"),
            serde_json::Value::Null
        );
    }

    /// 合格条件1: 提案の候補に折り方が付き、そのまま作品へ入れられる形になっている
    /// (作業27)。
    ///
    /// 出っぱり**6本**の骨格を使う。**実測して選んだ**: 出っぱり4本・5本の展開図では
    /// どの折り線も「その1本だけでは平らに畳めない」ので折り方が1手も見つからず
    /// (4候補すべて0手)、6本では4候補すべてに1〜2手が付いた。
    ///
    /// **2026-08-23に、探索の当たり外れへぶら下がっていた部分だけを書き直した**
    /// (`CLAUDE.md` §10.7.9、詳しくは `scratchpad/undo-proposal-test-report.md`)。
    /// 折り方が付いた候補にかける条件は**全件そのまま**残し、
    /// 「折り方が付いた候補が1件以上ある」という下限だけをやめている。
    #[test]
    fn proposal_candidates_carry_a_fold_plan_that_is_ready_to_use() {
        use std::collections::BTreeSet;
        let out = proposal_generate(star(6), A4ISH, 1, true).expect("候補が返るはず");
        assert!(!out.is_empty(), "候補が0件");
        let mut with_plan = 0usize;
        for c in &out {
            let Some(plan) = &c.fold_plan else { continue };
            with_plan += 1;
            let details = plan.details();
            assert_eq!(
                details.checked,
                details.steps.len(),
                "確かめた手数と手順の数が食い違う"
            );
            assert!(details.checked >= 1, "折り方が付いたのに手数が0");
            assert!(
                details.checked <= details.planned,
                "確かめた手数{}が見つけた手数{}を超えている",
                details.checked,
                details.planned
            );
            if plan.checked_to_finish() {
                assert_eq!(
                    details.checked, details.planned,
                    "最後まで確かめたのに手数が違う"
                );
            }
            let json = serde_json::to_value(plan).expect("折り方をJSONへ運べない");
            let expected_status = if plan.checked_to_finish() {
                "checked_to_finish"
            } else {
                "partial"
            };
            assert_eq!(json["status"], expected_status);
            assert!(
                json.get("checked_to_finish").is_none(),
                "書き換え可能な完成boolが残っている"
            );
            let ids: BTreeSet<u32> = details.steps.iter().map(|s| s.id).collect();
            assert_eq!(ids.len(), details.steps.len(), "手順の番号が重なっている");
            assert!(
                !ori3_cp::extract_faces(&details.cp).is_empty(),
                "折り込んだ展開図から面を取り出せない"
            );

            // 入れた後、立体で面が欠けず、紙の重なり順に同じ番号が二重に出ないこと。
            //
            // **隠さず書く**: この折り方が記録する重なり順は、実測すると
            // 面の番号順(`[0, 1, 2, …]`)そのものだった(24面・28面の4候補すべて)。
            // 画面の普通の折り操作で同じことを測ると `[2, 3, 0, 1, 5, 6, 7, 4]` と
            // 面の番号順にはならない。つまり提案の折り方が通る道
            // (`ori3_layers::collapse_precrease_network`)は**紙の重なり順を
            // 組み替えていない**(`scratchpad/propose-21-report.md` §6 と同じ限界)。
            // 直すのは `crates/ori3-layers` 側の仕事なので、ここでは
            // 「番号が二重にならない」ことだけを見る。詳しくは
            // `scratchpad/propose-27-29-report.md` §5。
            let mut store = DocumentStore::default();
            store.new_document(A4ISH).expect("新規作品を作れるはず");
            let mut view = store
                .apply_proposal(details.cp.clone(), details.steps.clone())
                .expect("確かめた折り方は入れられるはず");
            attach_replay(&mut view);
            let frame = view.frame.as_ref().expect("折り手順があるので立体が返る");
            assert_eq!(frame.faces.len(), view.faces.len(), "立体で面が欠けている");
            let ranks: BTreeSet<u32> = frame.faces.iter().map(|f| f.surface_rank).collect();
            assert_eq!(
                ranks.len(),
                frame.faces.len(),
                "重なり順に同じ番号が二重にある"
            );
            let order = frame_surface_rank_order(frame).expect("重なり順を取り出せるはず");
            let mut by_face_id: Vec<FaceId> = frame.faces.iter().map(|f| f.face).collect();
            by_face_id.sort_unstable();
            assert_eq!(
                order.len(),
                frame.faces.len(),
                "重なり順の件数が面の数と違う"
            );
            assert_eq!(
                {
                    let mut sorted = order.clone();
                    sorted.sort_unstable();
                    sorted
                },
                by_face_id,
                "重なり順に出てくる面が、立体の面とそろっていない"
            );
        }
        // ここには以前 `assert!(with_plan >= 1, "折り方が付いた候補が1件も無い")` があった。
        //
        // これは「折り方の探索が [`PLAN_BUDGET`] の壁時計の打ち切り(6,000ms)までに
        // 1手でも確かめられたか」に主張をぶら下げる下限で、`CLAUDE.md` §10.7.9 が
        // 禁じる「余裕0の境目」だった。実測(2026-08-23、最適化なし、この骨格):
        // 候補#0の探索は **6,943ms**、候補#2は **6,021ms** で、どちらも
        // **打ち切りに当たっていた**(旧`SearchStop::TimeCap`)。1手が先に見つかっていた
        // おかげで `with_plan >= 1` を満たしただけで、余裕は無い。
        // CIの計算機は手元より約3.6倍遅いので、打ち切りはもっと早く来る。
        //
        // そこで下限は置かず、「提案の展開図はそのまま作品へ入れられる」ことを
        // **探索を通らない相手**で必ず1回確かめる。折り方が付いた候補には
        // 上の繰り返しの条件がそのままかかり、`with_plan` は
        // 「見つかった分を全部見たか」の照合(この節の最後)にだけ使う。
        let ready_cp = out[0].cp.clone();
        assert!(
            !ori3_cp::extract_faces(&ready_cp).is_empty(),
            "生成された展開図から紙の面を取り出せない"
        );
        let mut store = DocumentStore::default();
        store.new_document(A4ISH).expect("新規作品を作れるはず");
        let mut view = store
            .apply_proposal(
                ready_cp.clone(),
                vec![
                    finish_step(0, TechniqueKind::Simple, None),
                    finish_step(1, TechniqueKind::Simple, None),
                ],
            )
            .expect("提案の展開図はそのまま入れられるはず");
        assert_eq!(view.doc.cp, ready_cp, "展開図を全て格納していない");
        assert_eq!(view.doc.sequence.len(), 2, "手順を全て格納していない");
        attach_replay(&mut view);
        let frame = view.frame.as_ref().expect("折り手順があるので立体が返る");
        assert_eq!(frame.faces.len(), view.faces.len(), "立体で面が欠けている");
        assert_eq!(
            frame
                .faces
                .iter()
                .map(|f| f.surface_rank)
                .collect::<BTreeSet<u32>>()
                .len(),
            frame.faces.len(),
            "重なり順に同じ番号が二重にある"
        );
        // 上の繰り返しが、折り方を持つ候補を1件も飛ばしていないことの証拠。
        // 「何件見つかったか」ではなく「見つかった分を全部見たか」を言うので、
        // 探索が何件に折り方を付けたかには左右されない。
        assert_eq!(
            with_plan,
            out.iter().filter(|c| c.fold_plan.is_some()).count(),
            "折り方が付いた候補を見落としている"
        );
    }

    /// 折り方を付けない呼び方では、折り方の計算をまったく行わない。
    #[test]
    fn proposal_generate_without_a_fold_plan_leaves_it_empty() {
        let out = proposal_generate(star(6), A4ISH, 1, false).expect("候補が返るはず");
        assert!(!out.is_empty());
        assert!(out.iter().all(|c| c.fold_plan.is_none()));
    }

    /// 画面(`plan_folds`)が使う時間の上限は **30,000ms(30秒)の安全弁**である。
    ///
    /// **2026-08-23に 6,000ms から変えた。緩めたのではなく、役目を変えた。**
    /// 6,000msのときは先端12本で候補4件のうち2件がこの打ち切りに当たり、
    /// **同じ入力でも機械の混み具合で答えが変わっていた**(`CLAUDE.md` §10.7.7)。
    /// いまは**計算量(`max_states` / `branch`)だけで切り**、
    /// 時間の上限は「何かがおかしくて終わらない」ときのためだけに残す。
    ///
    /// 根拠(最適化ありの実測、`PLAN_BUDGET` のコメント): いちばん重い先端12本の
    /// 待ち時間が **13.851秒**。その **2.2倍** が 30,000ms で、実測は上限の **46.2%**。
    /// 検査用の既定
    /// [`SearchWatchdog::MAX_MILLIS`](ori3_propose::SearchWatchdog::MAX_MILLIS)
    /// とは別の値であることも合わせて固定する。
    ///
    /// **値をぴったり固定したままにしてある**ので、次に根拠なく変えれば落ちる。
    #[test]
    fn plan_budget_keeps_the_screen_time_limit_as_a_safety_valve() {
        assert_eq!(
            PLAN_BUDGET.watchdog.max_millis, 30_000,
            "画面用の時間打切りを根拠なく変えた"
        );
        assert_ne!(
            PLAN_BUDGET.watchdog.max_millis,
            SearchWatchdog::MAX_MILLIS,
            "画面用の打切りが検査用の既定のままでは、待ち時間の見積もりが立たない"
        );
        assert_eq!(
            (
                PLAN_BUDGET.deterministic.max_states,
                PLAN_BUDGET.deterministic.branch,
            ),
            (2, 2),
            "計算量の上限を根拠なく変えた(先端12本で 2/2 は8手、1/2 は4手へ半減する)"
        );
    }

    /// 0ms watchdogは候補全体の専用Errになり、partial候補を1件も返さないこと。
    #[test]
    fn a_watchdog_aborted_plan_search_returns_no_partial_candidate() {
        let zero_watchdog = PlanBudget {
            watchdog: SearchWatchdog { max_millis: 0 },
            ..PLAN_BUDGET
        };
        let jobs = ProposalJobs::default();
        let job_id = ProposalJobId::from("zero-watchdog");
        let lease = jobs.start(job_id).expect("watchdog jobを登録できる");
        let result = generate_candidates(&star(4), &A4ISH, 1, true, zero_watchdog, &lease.job);
        assert_eq!(
            result,
            Err(ProposalGenerationError::SearchAborted(
                SearchAbort::WatchdogExpired
            )),
            "watchdogが候補Vecまたは通常の生成失敗へ化けた"
        );
        let counted = lease.job.snapshot();
        assert_eq!(counted.done, counted.total, "中断workerの完了数が欠けた");
        assert!(counted.total > 0, "watchdogを通る候補が作られていない");
        let terminal = lease.complete(ProposalPhase::Failed);
        assert_eq!(terminal.phase, ProposalPhase::Failed);
        assert_eq!(jobs.registered_count(), 0, "watchdog後にjobが残った");
    }

    #[test]
    fn a_cancelled_job_returns_no_candidate_and_is_pruned() {
        let jobs = ProposalJobs::default();
        let job_id = ProposalJobId::from("cancel-before-generate");
        let lease = jobs.start(job_id.clone()).expect("cancel jobを登録できる");
        jobs.cancel(&job_id).expect("生成前にcancelできる");
        let result = generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &lease.job);
        assert_eq!(
            result,
            Err(ProposalGenerationError::SearchAborted(
                SearchAbort::Cancelled
            )),
            "cancelが候補Vecまたは通常の生成失敗へ化けた"
        );
        let terminal = lease.complete(ProposalPhase::Cancelled);
        assert_eq!(terminal.phase, ProposalPhase::Cancelled);
        assert_eq!((terminal.done, terminal.total), (0, 0));
        assert_eq!(jobs.registered_count(), 0, "cancel後にjobが残った");
    }

    #[test]
    fn export_bytes_makes_svg_and_png() {
        use super::{ExportKind, ExportOptions, export_files};
        let doc = ori3_model::Document::new(A4ISH);
        let opts = ExportOptions {
            include_aux: true,
            png_long_side: 128,
        };
        let (svg, svg_issues) = export_files(&doc, ExportKind::CpSvg, opts).unwrap();
        assert!(svg_issues.is_empty());
        assert_eq!(svg.len(), 1);
        assert!(svg[0].0.is_empty(), "1つのファイルなので番号は付かない");
        let text = String::from_utf8(svg[0].1.clone()).unwrap();
        assert!(text.contains("viewBox=\"0 0 150 150\""), "{text}");

        let (png, png_issues) = export_files(&doc, ExportKind::CpPng, opts).unwrap();
        assert!(png_issues.is_empty());
        assert_eq!(&png[0].1[0..8], b"\x89PNG\r\n\x1a\n");

        // 点数が0など無理な指定は日本語のErr(パニックにしない)
        let bad = ExportOptions {
            include_aux: false,
            png_long_side: 0,
        };
        assert!(export_files(&doc, ExportKind::CpPng, bad).is_err());
    }

    #[test]
    fn fold_export_returns_parseable_bytes_and_structured_warnings() {
        use super::{ExportKind, ExportOptions, export_files};
        use ori3_export::fold::{FoldIssueCode, parse_fold_1_2};

        let mut doc = ori3_model::Document::new(A4ISH);
        ori3_cp::insert_segment(&mut doc.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Aux);
        let options = ExportOptions {
            include_aux: false,
            png_long_side: 128,
        };

        let (files, issues) =
            export_files(&doc, ExportKind::FoldJson, options).expect("FOLDを書き出せる");
        assert_eq!(files.len(), 1);
        assert!(files[0].0.is_empty(), "単一ファイルは既存の空suffixを使う");
        let text = std::str::from_utf8(&files[0].1).expect("FOLD JSONはUTF-8");
        parse_fold_1_2(text).expect("製品writerのFOLDを製品parserで読める");
        let aux = issues
            .iter()
            .find(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
            .expect("AuxからUへの変更を警告する");
        assert!(!aux.path.is_empty());
        assert!(aux.original_value.is_some());
        let wire = serde_json::to_value(aux).expect("FoldIssueをそのままIPCへ渡せる");
        assert_eq!(
            wire.get("path").and_then(serde_json::Value::as_str),
            Some(aux.path.as_str())
        );
        assert!(wire.get("original_value").is_some());
    }

    #[test]
    fn fold_export_uses_the_existing_single_file_writer() {
        use super::{ExportKind, ExportOptions, export_files, write_export_files};

        let doc = ori3_model::Document::new(A4ISH);
        let options = ExportOptions {
            include_aux: false,
            png_long_side: 128,
        };
        let (files, _) =
            export_files(&doc, ExportKind::FoldJson, options).expect("FOLDを書き出せる");
        let dir = std::env::temp_dir().join(format!(
            "ori3_fold_export_{}_{}",
            std::process::id(),
            super::EXPORT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("検査用directoryを作れる");
        let target = dir.join("roundtrip.fold");
        write_export_files(&target, &files).expect("既存writerで単一FOLDを書ける");
        let text = std::fs::read_to_string(&target).expect("書いたFOLDを読める");
        ori3_export::fold::parse_fold_1_2(&text).expect("書いたFOLDをparseできる");
        std::fs::remove_dir_all(&dir).expect("検査用directoryを片付けられる");
    }

    #[test]
    fn fold_export_conversion_failure_never_touches_an_existing_file() {
        use super::{ExportKind, ExportOptions, export_files};

        let mut doc = ori3_model::Document::new(A4ISH);
        doc.paper.width_mm = f64::NAN;
        let options = ExportOptions {
            include_aux: false,
            png_long_side: 128,
        };
        let dir = std::env::temp_dir().join(format!(
            "ori3_fold_export_failure_{}_{}",
            std::process::id(),
            super::EXPORT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("検査用directoryを作れる");
        let target = dir.join("sentinel.fold");
        let sentinel = b"previous completed FOLD";
        std::fs::write(&target, sentinel).expect("sentinelを書ける");

        assert_eq!(
            export_files(&doc, ExportKind::FoldJson, options).expect_err("非有限値を拒否する"),
            "この作品はFOLD 1.2 限定として書き出せません。作品の内容を確認してください。"
        );
        assert_eq!(std::fs::read(&target).expect("sentinelを読める"), sentinel);
        std::fs::remove_dir_all(&dir).expect("検査用directoryを片付けられる");
    }

    /// 折り図は手順が無いと書き出せず、理由を日本語で返す(EXP-003 / EXP-004)。
    #[test]
    fn diagram_export_needs_steps() {
        use super::{ExportKind, ExportOptions, export_files};
        let doc = ori3_model::Document::new(A4ISH);
        let opts = ExportOptions {
            include_aux: true,
            png_long_side: 128,
        };
        for kind in [ExportKind::DiagramPdf, ExportKind::DiagramSvg] {
            let err = export_files(&doc, kind, opts).unwrap_err();
            assert!(err.contains("折り手順がまだありません"), "err={err}");
        }
    }

    /// 画像の大きさに負の数などを入れても、英語のエラーではなく日本語で理由が出る。
    #[test]
    fn a_bad_png_size_is_a_japanese_error() {
        use super::{ExportKind, ExportOptions, export_files, png_long_side};
        for bad in [-5, -1, 0] {
            let err = png_long_side(bad).unwrap_err();
            assert!(err.contains("1以上"), "err={err}");
            assert!(err.contains(&bad.to_string()), "指定値が出ない: {err}");
        }
        let huge = ori3_export::MAX_LONG_SIDE_PX as i64 + 1;
        let err = png_long_side(huge).unwrap_err();
        assert!(err.contains("までにしてください"), "err={err}");
        assert_eq!(png_long_side(1), Ok(1));
        assert_eq!(png_long_side(2048), Ok(2048));

        // 書き出し口から見ても同じ(パニックにならない)
        let doc = ori3_model::Document::new(A4ISH);
        let opts = ExportOptions {
            include_aux: false,
            png_long_side: -5,
        };
        let err = export_files(&doc, ExportKind::CpPng, opts).unwrap_err();
        assert!(err.contains("1以上"), "err={err}");
    }

    /// 折り図の画像はページごとに番号を足した名前になる。
    #[test]
    fn diagram_pages_get_numbered_file_names() {
        use super::export_target;
        use std::path::Path;
        let base = Path::new("C:/作品/鶴.svg");
        assert_eq!(export_target(base, ""), base.to_path_buf());
        assert_eq!(
            export_target(base, "-02"),
            Path::new("C:/作品/鶴-02.svg").to_path_buf()
        );
    }

    #[test]
    fn failed_multi_svg_export_never_deletes_the_previous_file() {
        use super::write_export_files;
        let dir = std::env::temp_dir().join(format!("ori3_export_rollback_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("折り図.svg");
        let first = dir.join("折り図-01.svg");
        let second = dir.join("折り図-02.svg");
        std::fs::write(&first, "前からある1ページ目").unwrap();
        // 2ページ目の完成名をディレクトリにして、名前の切り替えだけを失敗させる。
        // 一時ファイルへの全ページ書き出しは通るため、巻き戻し経路を検査できる。
        std::fs::create_dir(&second).unwrap();
        let files = vec![
            ("-01".to_string(), "今回の1ページ目".as_bytes().to_vec()),
            ("-02".to_string(), "今回の2ページ目".as_bytes().to_vec()),
        ];

        assert!(write_export_files(&base, &files).is_err());
        assert!(first.exists(), "上書き前からあるファイルを消している");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// たわみの指定が無い/切ってあるときは何も返さず、入れたときだけ網が返る
    /// (SIM-012。既存の呼び出しの動きは変わらない)。
    #[test]
    fn soft_mesh_only_when_enabled() {
        use super::soft_mesh;
        use ori3_soft::SoftSettings;
        let mut doc = ori3_model::Document::new(A4ISH);
        ori3_cp::insert_segment(&mut doc.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
        let faces = ori3_cp::extract_faces(&doc.cp);
        let mut frame = ori3_layers::replay_with_faces(&doc, &faces, 0, 1.0).frame;
        assert_eq!(frame.faces.len(), 2, "soft順位を比較する2面");
        let face_count = frame.faces.len();
        for (index, face) in frame.faces.iter_mut().enumerate() {
            face.layer = u32::try_from(index).unwrap();
            face.surface_rank = u32::try_from(face_count - index - 1).unwrap();
        }
        let original_layers = frame
            .faces
            .iter()
            .map(|face| (face.face, face.layer))
            .collect::<HashMap<_, _>>();
        let display_ranks = frame
            .faces
            .iter()
            .map(|face| (face.face, face.surface_rank))
            .collect::<HashMap<_, _>>();

        assert!(soft_mesh(&doc.cp, &faces, &frame, true, None).is_none());
        let off = SoftSettings::default();
        assert!(!off.enabled, "たわみの既定はオフ");
        assert!(soft_mesh(&doc.cp, &faces, &frame, true, Some(&off)).is_none());

        let on = SoftSettings {
            enabled: true,
            ..SoftSettings::default()
        };
        assert!(
            soft_mesh(&doc.cp, &faces, &frame, false, Some(&on)).is_none(),
            "完全順列でも幾何proofが無ければsoftへmaterial seedを渡さない"
        );
        let mesh = soft_mesh(&doc.cp, &faces, &frame, true, Some(&on)).expect("網が返るはず");
        assert!(!mesh.triangles.is_empty(), "三角形が無い");
        assert_eq!(mesh.triangles.len(), mesh.triangle_faces.len());
        assert_eq!(mesh.triangles.len(), mesh.triangle_layers.len());
        for (&face, &layer) in mesh.triangle_faces.iter().zip(&mesh.triangle_layers) {
            assert_eq!(layer, display_ranks[&face], "softもsurface rankを層に使う");
        }
        assert_eq!(
            frame
                .faces
                .iter()
                .map(|face| (face.face, face.layer))
                .collect::<HashMap<_, _>>(),
            original_layers,
            "soft入力の複製で論理layerを変えない"
        );
        // 分割しているので、元の面(1枚=三角形2つ)より細かくなる
        assert!(mesh.triangles.len() > 2, "分割されていない");
    }

    #[test]
    fn saved_layer_order_stamp_updates_only_the_logical_layer() {
        let mut frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 10,
                    polygon: Vec::new(),
                    layer: 7,
                    surface_rank: 0,
                    mirrored: false,
                },
                Face3D {
                    face: 20,
                    polygon: Vec::new(),
                    layer: 3,
                    surface_rank: 1,
                    mirrored: true,
                },
            ],
            warnings: Vec::new(),
        };
        assert!(stamp_saved_layer_order(&mut frame, Some(&[20, 10])));
        assert_eq!(frame.faces[0].surface_rank, 0);
        assert_eq!(frame.faces[1].surface_rank, 1);
        assert_eq!(
            frame
                .faces
                .iter()
                .map(|face| face.layer)
                .collect::<Vec<_>>(),
            vec![1, 0],
            "保存順は後続手順用layerだけを更新する"
        );

        let before = frame
            .faces
            .iter()
            .map(|face| (face.layer, face.surface_rank))
            .collect::<Vec<_>>();
        assert!(!stamp_saved_layer_order(&mut frame, Some(&[10, 10])));
        assert_eq!(
            frame
                .faces
                .iter()
                .map(|face| (face.layer, face.surface_rank))
                .collect::<Vec<_>>(),
            before,
            "重複した順序ではcanonical fallbackを変えない"
        );
        assert!(!stamp_saved_layer_order(&mut frame, None));
    }

    #[test]
    fn pose_overlap_uses_canonical_then_untrusted_fallback() {
        let frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 10,
                    polygon: Vec::new(),
                    layer: 0,
                    surface_rank: 1,
                    mirrored: false,
                },
                Face3D {
                    face: 20,
                    polygon: Vec::new(),
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                },
            ],
            warnings: Vec::new(),
        };
        let fallback = [10, 20];
        let from_canonical = pose_overlap_order(&frame, &fallback, true);
        assert_eq!(from_canonical.order, vec![20, 10]);
        assert!(from_canonical.authoritative);

        let seed_without_proof = pose_overlap_order(&frame, &fallback, false);
        assert_eq!(seed_without_proof.order, fallback);
        assert!(
            !seed_without_proof.authoritative,
            "完全順列だけではpose PBDのauthorityにしない"
        );

        let mut invalid_rank = frame;
        invalid_rank.faces[0].surface_rank = 0;
        invalid_rank.faces[1].surface_rank = 0;
        let untrusted = pose_overlap_order(&invalid_rank, &fallback, true);
        assert_eq!(untrusted.order, fallback);
        assert!(!untrusted.authoritative);
    }

    #[test]
    fn nonfinite_pose_fallback_cannot_keep_surface_authority() {
        let doc = ori3_model::Document::new(A4ISH);
        let faces = ori3_cp::extract_faces(&doc.cp);
        let mut result = ori3_rigid::solve(&doc.cp, &faces, &[], None);
        assert!(pose_result_is_finite(&result));
        assert!(usable_pose_surface_order(true, &result));

        result.frame.faces[0].polygon[0][0] = f64::NAN;
        assert!(
            !usable_pose_surface_order(true, &result),
            "motionがauthorityを報告してもnonfinite fallback前のproofを継承しない"
        );
    }

    #[test]
    fn guard_converts_panic_to_err() {
        let r: Result<(), String> = guard(AssertUnwindSafe(|| panic!("爆発した")));
        let err = r.unwrap_err();
        assert!(err.contains("内部エラー"), "err={err}");
        assert!(err.contains("爆発した"), "err={err}");
    }

    #[test]
    fn guard_passes_through_ok_and_err() {
        assert_eq!(guard(AssertUnwindSafe(|| Ok(42))), Ok(42));
        assert_eq!(
            guard(AssertUnwindSafe(|| Err::<(), _>("だめ".to_string()))),
            Err("だめ".to_string())
        );
    }

    #[test]
    fn sequence_operation_keeps_spatial_hit_and_legacy_fold_fields() {
        let value = serde_json::json!({
            "type": "PreviewFoldThrough",
            "up_to": 1,
            "line": [[0.0, 0.0], [1.0, 0.0]],
            "keep_side_point": [0.0, 1.0],
            "target_layers": null,
            "direction": "Up",
            "spatial": {
                "from": [0.5, 0.25, -0.25],
                "to": [0.5, 0.5, -0.25],
                "grab_face": 1,
                "mode": "flap"
            }
        });
        let (operation, spatial) = super::parse_sequence_operation(value).expect("読み取れる");
        assert!(matches!(
            operation,
            ori3_model::SeqOp::PreviewFoldThrough { up_to: 1, .. }
        ));
        let spatial = spatial.expect("立体の当たり点");
        assert_eq!(spatial.from, [0.5, 0.25, -0.25]);
        assert_eq!(spatial.to, [0.5, 0.5, -0.25]);
        assert_eq!(spatial.grab_face, 1);
    }

    const FOLD_ALL_PERCENTAGES: [f64; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];
    const FOLD_ALL_FIXTURES: [(&str, &str, usize); 3] = [
        (
            "折り鶴",
            include_str!("../../../../crates/ori3-rigid/tests/fixtures/check-crane.ori3"),
            43,
        ),
        (
            "鳥の基本形",
            include_str!("../../../../crates/ori3-rigid/tests/fixtures/check-bird-base.ori3"),
            18,
        ),
        (
            "やっこさん",
            include_str!("../../../../crates/ori3-rigid/tests/fixtures/check-yakko.ori3"),
            20,
        ),
    ];

    #[derive(Debug, PartialEq, Eq)]
    struct FoldAllFaceSignature {
        face: FaceId,
        layer: u32,
        surface_rank: u32,
        mirrored: bool,
        polygon: Vec<[u64; 3]>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct FoldAllSignature {
        frame: Vec<FoldAllFaceSignature>,
        warnings: Vec<String>,
        converged: bool,
        angles: Vec<(EdgeId, u64)>,
        closure_rms: u64,
        best_effort: bool,
        relaxations: Vec<(EdgeId, u64, u64, u64)>,
        iterations: u32,
        requested_percent: u64,
        requested_angles: Vec<(EdgeId, u64)>,
        next_warm_seed: Vec<(EdgeId, u64)>,
        suspect_hinges: Vec<EdgeId>,
        contact_detected: bool,
        flat_fold_violations: Vec<VertexId>,
        layer_order: FoldAllLayerOrder,
    }

    fn fold_all_inputs(text: &str) -> (Document, Vec<ori3_cp::Face>) {
        let document = crate::store::parse_document(text)
            .expect("一斉折りfixtureを製品readerで読める")
            .document;
        let faces = ori3_cp::extract_faces(&document.cp);
        (document, faces)
    }

    fn sorted_angle_bits(angles: &HashMap<EdgeId, f64>) -> Vec<(EdgeId, u64)> {
        let mut sorted: Vec<_> = angles
            .iter()
            .map(|(&hinge, &angle)| (hinge, angle.to_bits()))
            .collect();
        sorted.sort_unstable_by_key(|(hinge, _)| *hinge);
        sorted
    }

    fn driver_bits(drivers: &[Driver]) -> Vec<(EdgeId, u64)> {
        drivers
            .iter()
            .map(|driver| (driver.hinge, driver.target_angle_deg.to_bits()))
            .collect()
    }

    fn fold_all_signature(outcome: &super::FoldAllPreviewOutcome) -> FoldAllSignature {
        FoldAllSignature {
            frame: outcome
                .result
                .frame
                .faces
                .iter()
                .map(|face| FoldAllFaceSignature {
                    face: face.face,
                    layer: face.layer,
                    surface_rank: face.surface_rank,
                    mirrored: face.mirrored,
                    polygon: face
                        .polygon
                        .iter()
                        .map(|point| point.map(f64::to_bits))
                        .collect(),
                })
                .collect(),
            warnings: outcome.result.frame.warnings.clone(),
            converged: outcome.result.converged,
            angles: sorted_angle_bits(&outcome.result.angles),
            closure_rms: outcome.result.closure_rms.to_bits(),
            best_effort: outcome.result.best_effort,
            relaxations: outcome
                .result
                .relaxations
                .iter()
                .map(|relaxation| {
                    (
                        relaxation.hinge,
                        relaxation.target_angle_deg.to_bits(),
                        relaxation.actual_angle_deg.to_bits(),
                        relaxation.delta_deg.to_bits(),
                    )
                })
                .collect(),
            iterations: outcome.result.iterations,
            requested_percent: outcome.requested_percent.to_bits(),
            requested_angles: driver_bits(&outcome.requested_angles),
            next_warm_seed: driver_bits(&outcome.next_warm_seed),
            suspect_hinges: outcome.suspect_hinges.clone(),
            contact_detected: outcome.contact_detected,
            flat_fold_violations: outcome.flat_fold_violations.clone(),
            layer_order: outcome.layer_order,
        }
    }

    fn assert_fold_all_finite_and_complete(
        name: &str,
        document: &Document,
        faces: &[ori3_cp::Face],
        percent: f64,
        expected_hinges: usize,
        outcome: &super::FoldAllPreviewOutcome,
    ) {
        assert_eq!(outcome.requested_percent, percent, "{name} {percent}%");
        assert_eq!(
            outcome.requested_angles.len(),
            expected_hinges,
            "{name} {percent}%: 希望角の本数"
        );
        assert_eq!(
            outcome.next_warm_seed.len(),
            expected_hinges,
            "{name} {percent}%: 次回出発角の本数"
        );
        assert!(
            outcome
                .requested_angles
                .windows(2)
                .all(|pair| pair[0].hinge < pair[1].hinge),
            "{name} {percent}%: 希望角が辺ID順でない"
        );
        assert!(
            outcome
                .next_warm_seed
                .windows(2)
                .all(|pair| pair[0].hinge < pair[1].hinge),
            "{name} {percent}%: 次回出発角が辺ID順でない"
        );

        let kinds: HashMap<_, _> = document
            .cp
            .edges
            .iter()
            .map(|edge| (edge.id, edge.kind))
            .collect();
        for driver in &outcome.requested_angles {
            let expected = match kinds[&driver.hinge] {
                EdgeKind::Mountain => 180.0 * percent / 100.0,
                EdgeKind::Valley => -180.0 * percent / 100.0,
                EdgeKind::Border | EdgeKind::Aux => {
                    panic!("{name} {percent}%: 輪郭・補助線を希望角に含めた")
                }
            };
            assert_eq!(
                driver.target_angle_deg, expected,
                "{name} {percent}%: 辺{}の符号または角度",
                driver.hinge
            );
        }

        assert_eq!(
            outcome.result.frame.faces.len(),
            faces.len(),
            "{name} {percent}%: 面の欠落"
        );
        let source_faces: HashMap<_, _> = faces.iter().map(|face| (face.id, face)).collect();
        for face in &outcome.result.frame.faces {
            let source = source_faces
                .get(&face.face)
                .unwrap_or_else(|| panic!("{name} {percent}%: 未知の面{}", face.face));
            assert_eq!(
                face.polygon.len(),
                source.vertices.len(),
                "{name} {percent}%: 面{}の頂点欠落",
                face.face
            );
            assert!(
                face.polygon.iter().flatten().all(|value| value.is_finite()),
                "{name} {percent}%: 非finite頂点"
            );
            assert_eq!((face.layer, face.surface_rank), (0, 0));
        }
        assert!(outcome.result.closure_rms.is_finite());
        assert!(
            outcome
                .result
                .angles
                .values()
                .all(|angle| angle.is_finite())
        );
        assert!(outcome.result.relaxations.iter().all(|relaxation| {
            relaxation.target_angle_deg.is_finite()
                && relaxation.actual_angle_deg.is_finite()
                && relaxation.delta_deg.is_finite()
        }));
        assert!(
            outcome
                .next_warm_seed
                .iter()
                .all(|driver| driver.target_angle_deg.is_finite())
        );
        assert_eq!(
            outcome.layer_order,
            FoldAllLayerOrder::UnavailableWithoutSequence
        );
        assert!(
            outcome
                .result
                .frame
                .warnings
                .iter()
                .any(|warning| warning == ori3_rigid::FOLD_ALL_LAYER_ORDER_WARNING),
            "{name} {percent}%: 重なり順不明の警告がない"
        );
        if percent == 0.0 {
            assert!(
                outcome
                    .result
                    .angles
                    .values()
                    .all(|angle| angle.abs() <= 1e-9),
                "{name}: 0%の実角が1e-9度を超えた"
            );
        }
    }

    fn flat_vertex_distance(
        document: &Document,
        faces: &[ori3_cp::Face],
        outcome: &super::FoldAllPreviewOutcome,
    ) -> f64 {
        let source_faces: HashMap<_, _> = faces.iter().map(|face| (face.id, face)).collect();
        let vertices: HashMap<_, _> = document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect();
        outcome
            .result
            .frame
            .faces
            .iter()
            .flat_map(|face| {
                let source = source_faces[&face.face];
                face.polygon
                    .iter()
                    .zip(source.vertices.iter())
                    .map(|(actual, vertex)| {
                        let expected = vertices[vertex];
                        let dx = actual[0] - expected[0];
                        let dy = actual[1] - expected[1];
                        let dz = actual[2];
                        (dx * dx + dy * dy + dz * dz).sqrt()
                    })
            })
            .fold(0.0_f64, f64::max)
    }

    #[test]
    fn fold_all_three_fixtures_are_finite_flat_and_bit_deterministic() {
        let mut maximum_flat_distance = 0.0_f64;
        for (name, text, expected_hinges) in FOLD_ALL_FIXTURES {
            let (document, faces) = fold_all_inputs(text);
            let document_before = document.clone();
            for percent in FOLD_ALL_PERCENTAGES {
                let first = fold_all_preview_outcome(&document, &faces, percent, None)
                    .unwrap_or_else(|error| panic!("{name} {percent}%: {error}"));
                assert_fold_all_finite_and_complete(
                    name,
                    &document,
                    &faces,
                    percent,
                    expected_hinges,
                    &first,
                );
                if percent == 0.0 {
                    maximum_flat_distance =
                        maximum_flat_distance.max(flat_vertex_distance(&document, &faces, &first));
                }
                let expected = fold_all_signature(&first);

                for repetition in 2..=10 {
                    let repeated = fold_all_preview_outcome(&document, &faces, percent, None)
                        .unwrap_or_else(|error| {
                            panic!("{name} {percent}% {repetition}回目: {error}")
                        });
                    assert_fold_all_finite_and_complete(
                        name,
                        &document,
                        &faces,
                        percent,
                        expected_hinges,
                        &repeated,
                    );
                    if percent == 0.0 {
                        maximum_flat_distance = maximum_flat_distance
                            .max(flat_vertex_distance(&document, &faces, &repeated));
                    }
                    assert_eq!(
                        fold_all_signature(&repeated),
                        expected,
                        "{name} {percent}% {repetition}回目がbit単位で不一致"
                    );
                }
            }
            assert_eq!(document, document_before, "{name}: Documentを変更した");
        }

        println!("一斉折り0%の全頂点最大距離: {maximum_flat_distance:.17e}");
        // 2026-08-24、指定3標本×0%×10回の実測最大距離は0.0だった。
        // 0%は恒等変換の経路だが、CPU差の最下位丸めに1e-12の余裕を持たせる。
        // 1e-9の位置ずれは十分検出でき、実測値を境目そのものにはしていない。
        const FLAT_VERTEX_TOLERANCE: f64 = 1e-12;
        assert!(
            maximum_flat_distance <= FLAT_VERTEX_TOLERANCE,
            "0%が平面と一致しない: 最大{maximum_flat_distance:e} > {FLAT_VERTEX_TOLERANCE:e}"
        );
    }

    #[test]
    fn fold_all_wire_contract_is_temporary_and_explicit_about_layer_order() {
        use std::collections::BTreeSet;

        let (document, faces) = fold_all_inputs(FOLD_ALL_FIXTURES[1].1);
        let outcome =
            fold_all_preview_outcome(&document, &faces, 50.0, None).expect("鳥の基本形50%を返す");
        let value = serde_json::to_value(&outcome).expect("一時姿勢をJSONへ運べる");
        let object = value.as_object().expect("一時姿勢はobject");
        let actual: BTreeSet<_> = object.keys().map(String::as_str).collect();
        let expected = BTreeSet::from([
            "angles",
            "best_effort",
            "closure_rms",
            "contact_detected",
            "converged",
            "flat_fold_violations",
            "frame",
            "iterations",
            "layer_order",
            "next_warm_seed",
            "relaxations",
            "requested_angles",
            "requested_percent",
            "suspect_hinges",
        ]);
        assert_eq!(actual, expected, "wire fieldの増減は明示的に扱う");
        assert_eq!(value["layer_order"], "unavailable_without_sequence");
        assert_eq!(value["requested_percent"], 50.0);
        assert!(value["requested_angles"].is_array());
        assert!(value["next_warm_seed"].is_array());
        assert!(object.get("document").is_none());
        assert!(object.get("sequence").is_none());
        assert!(object.get("undo").is_none());
        assert!(object.get("surface_order").is_none());
    }

    #[test]
    fn non_flat_foldable_fold_all_returns_a_pose_and_warning_instead_of_blocking() {
        let mut document = Document::new(A4ISH);
        let center = [0.5, 0.5];
        for (endpoint, kind) in [
            ([1.0, 0.5], EdgeKind::Mountain),
            ([0.682, 1.0], EdgeKind::Valley),
            ([0.0, 0.5], EdgeKind::Mountain),
            ([0.5, 0.0], EdgeKind::Valley),
        ] {
            ori3_cp::insert_segment(&mut document.cp, center, endpoint, kind);
        }
        let local_violations = ori3_cp::local_violations(&document.cp);
        assert!(!local_violations.is_empty(), "非平坦標本になっていない");
        let faces = ori3_cp::extract_faces(&document.cp);
        let outcome = fold_all_preview_outcome(&document, &faces, 100.0, None)
            .expect("平らにたためなくても姿勢を返す");
        assert!(outcome.result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        }));
        assert!(!outcome.flat_fold_violations.is_empty());
        assert!(
            outcome
                .result
                .frame
                .warnings
                .iter()
                .any(|warning| warning == super::FOLD_ALL_FLAT_FOLD_WARNING)
        );
    }

    #[test]
    fn fold_all_at_once_three_samples_timing() {
        let mut total = Duration::ZERO;
        let mut maximum = Duration::ZERO;
        let mut maximum_context = ("", 0.0_f64, 0_usize);
        let mut calls = 0_u32;

        for (name, text, _) in FOLD_ALL_FIXTURES {
            let (document, faces) = fold_all_inputs(text);
            let warmup = fold_all_preview_outcome(&document, &faces, 0.0, None)
                .unwrap_or_else(|error| panic!("{name} warm-up: {error}"));
            let mut warm_seed = Some(warmup.next_warm_seed);
            let mut sample_total = Duration::ZERO;
            let mut sample_maximum = Duration::ZERO;

            for sweep in 1..=10 {
                for percent in FOLD_ALL_PERCENTAGES {
                    let started = Instant::now();
                    let outcome =
                        fold_all_preview_outcome(&document, &faces, percent, warm_seed.take())
                            .unwrap_or_else(|error| {
                                panic!("{name} {percent}% sweep{sweep}: {error}")
                            });
                    let elapsed = started.elapsed();
                    warm_seed = Some(outcome.next_warm_seed);
                    sample_total += elapsed;
                    sample_maximum = sample_maximum.max(elapsed);
                    total += elapsed;
                    calls += 1;
                    if elapsed > maximum {
                        maximum = elapsed;
                        maximum_context = (name, percent, sweep);
                    }
                }
            }

            println!(
                "一斉折り性能 {name}: 50回 平均={:.3}ms 最大={:.3}ms",
                sample_total.as_secs_f64() * 1000.0 / 50.0,
                sample_maximum.as_secs_f64() * 1000.0,
            );
        }

        let average_ms = total.as_secs_f64() * 1000.0 / f64::from(calls);
        let maximum_ms = maximum.as_secs_f64() * 1000.0;
        println!(
            "一斉折り性能 全150回: 平均={average_ms:.3}ms 最大={maximum_ms:.3}ms（{} {}% sweep{}）",
            maximum_context.0, maximum_context.1, maximum_context.2
        );

        if !cfg!(debug_assertions) {
            const FRAME_BUDGET: Duration = Duration::from_millis(33);
            assert!(
                maximum <= FRAME_BUDGET,
                "release最大{maximum_ms:.3}msが33msを超えた。精度を下げず停止して報告する"
            );
        }
    }
}
