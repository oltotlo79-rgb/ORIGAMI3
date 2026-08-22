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
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::autosave;
use crate::store::{
    DocumentStore, DocumentView, SpatialFoldSpec, add_penetration_warning_for_intersections,
    apply_layer_order_display_settings, attach_replay, filter_penetration_warnings,
    flat_fold_notice_violations, frame_surface_rank_order, pose_flat_fold_notice_intersects,
    prevent_replay_overlap_if_authoritative, replay_flat_fold_notice_violations,
    replay_surface_rank_order,
};
use ori3_export::{CpSvgOptions, cp_png, cp_svg, diagram_pdf, diagram_svg_pages};
use ori3_model::{
    CreasePattern, DisplaySettings, Document, Driver, EdgeId, EditOp, FaceId, FinishSoftSettings,
    FoldStep, Frame3D, Paper, SeqOp, TechniqueKind, VertexId,
};
use ori3_propose::{
    CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, LeafSite, Packing,
    PoseScan, SearchBudget, Skeleton, TipSite, VerifiedPlan, body_on_paper, generate, pack,
    search_to_completion, verify_search_completion,
};
use ori3_soft::{SoftMesh, SoftSettings};

/// 複数ファイル書き出し用の同名一時ファイルを区別する連番。
static EXPORT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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
fn lock<'a>(state: &'a State<'_, Mutex<DocumentStore>>) -> MutexGuard<'a, DocumentStore> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// DocumentViewを返す操作の共通後処理: 手順を最新ステップまで自動再生して
/// 立体・飛ばした手順・警告をビューへ載せる(SEQ-004)。
///
/// 設計規約: ロック中に重い計算をしない。`f` の中で取ったロックは `f` を抜けた時点で
/// 解放されているので、再生(面400・10手順でrelease約23ms)はロックの外で走る。
fn store_view_pose_angles(state: &State<'_, Mutex<DocumentStore>>, view: &DocumentView) {
    if view.frame.is_some() && view.angles.values().all(|angle| angle.is_finite()) {
        lock(state).store_pose_angles(view.angles.clone());
    }
}

fn view_command(
    state: &State<'_, Mutex<DocumentStore>>,
    f: impl FnOnce() -> Result<DocumentView, String>,
) -> Result<DocumentView, String> {
    let mut view = f()?; // ここでロックは解放済み
    attach_replay(&mut view);
    store_view_pose_angles(state, &view);
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
        view_command(&state, || lock(&state).open(Path::new(&path)))
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
        store_view_pose_angles(&state, &view);
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
    guard(AssertUnwindSafe(|| {
        let (mut operation, spatial) = parse_sequence_operation(op)?;
        view_command(&state, || {
            let mut store = lock(&state);
            let document = store.export_inputs();
            record_finish_soft(&mut operation, &document.display);
            store.apply_seq_with_spatial(operation, spatial)
        })
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
    hard: Vec<Driver>,
    preferred: Option<Vec<Driver>>,
    soft: Option<SoftSettings>,
    warm_seed: Option<Vec<Driver>>,
    up_to: usize,
    t: f64,
) -> Result<PoseOutcome, String> {
    guard(AssertUnwindSafe(|| {
        let (doc, faces, stored_warm, overlap_enabled, penetration_enabled) =
            lock(&state).pose_inputs(); // 複製のみ、即ロック解放
        let soft = display_soft_settings(&doc, up_to, t, soft);
        let cp = &doc.cp;
        let saved_order = ori3_layers::saved_layer_order_at(&doc, &faces, up_to, t);
        let preferred = preferred.unwrap_or_default();
        // 同じ辺が両方にあれば、現在操作中のhardを後から入れて優先する。
        // warm_seedは出発角であって要求ではないため含めない。
        let requested_targets: Vec<Driver> = preferred.iter().chain(&hard).cloned().collect();
        let explicit_warm: Option<HashMap<EdgeId, f64>> = warm_seed.map(|seed| {
            seed.into_iter()
                .map(|driver| (driver.hinge, driver.target_angle_deg))
                .collect()
        });
        if explicit_warm
            .as_ref()
            .is_some_and(|seed| seed.values().any(|angle| !angle.is_finite()))
        {
            return Err("追従計算の出発角に有限でない値があります".to_string());
        }
        let warm = explicit_warm.as_ref().or(stored_warm.as_ref());
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
        let motion = ori3_rigid::solve_motion_with_contact_options(
            cp,
            &faces,
            &hard,
            targets.as_ref(),
            warm,
            pose_motion_contact_options(overlap_enabled, penetration_enabled),
        );
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
            lock(&state).store_pose_angles(result.angles.clone()); // 短いロックで書き戻し
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
/// 出っぱり4〜7本の骨格で、候補4件ぶんをまとめて計った時間と、
/// 出っぱり6本のときに見つかった手数(debugビルド)。
///
/// | `max_states` / `branch` | 4本 | 5本 | 6本 | 7本 | 6本で見つかった手数 |
/// |---|---:|---:|---:|---:|---|
/// | 1 / 3 | 2.4秒 | 9.4秒 | 6.5秒 | 6.1秒 | `[1, 1, 1, 1]` |
/// | **2 / 2** | **5.1秒** | **10.2秒** | **9.7秒** | **7.4秒** | **`[2, 1, 2, 1]`** |
/// | 2 / 3 | 5.1秒 | 13.5秒 | 12.7秒 | 7.9秒 | `[2, 1, 2, 1]` |
/// | 4 / 3 | 5.2秒 | 21.5秒 | 21.5秒 | 9.9秒 | `[3, 1, 2, 1]` |
///
/// `4 / 3` は `2 / 2` の2倍以上かかるのに、増える手は候補1件で1手だけだった。
/// `1 / 3` は速いが、どの候補も1手で止まる。**2 / 2** を採る。
///
/// ## `max_millis`(壁時計時間の上限)の決め方(実測、2026-08-22)
///
/// [`SearchBudget::MAX_MILLIS`](ori3_propose::SearchBudget::MAX_MILLIS)(240,000ms)は
/// **検査用の固定標本(既定12状態)を切らないための値**で、利用者が画面で
/// 「提案して」を押してから待ってよい時間としては長すぎる
/// (`scratchpad/propose-search-subset-report.md` §16.7.5)。この製品用の予算は
/// `max_states = 2` / `branch = 2` で、検査の12状態よりずっと軽いので、
/// 別の値を`max_millis`へ個別に入れる。
///
/// 最適化ありでの実測(この製品用の上限・分岐で、`scratchpad/search-budget-report.md`
/// に詳細)。
///
/// | 標本 | 1回の探索(最適化あり) |
/// |---|---:|
/// | 折り鶴 | **2.500秒** |
/// | 鳥の基本形 | 0.825秒 |
/// | やっこさん | 0.102秒 |
///
/// いちばん重い折り鶴2.500秒の**約2.4倍の余裕**を取り、**6,000ms(6秒)**とした。
/// 画面は候補を最大4件計算するので、最悪の待ち時間は 6秒 × 4 = 24秒 になる。
/// 打ち切っても操作は止まらない: 打ち切った時点での最善を返し、画面には
/// 「途中に注意があります」(`ProposalFoldPlanState::Partial`、手数は別表示)と出る
/// (`apps/desktop/src/components/dialogs/ProposalWizard.tsx::foldPlanLabel`)。
const PLAN_BUDGET: SearchBudget = SearchBudget {
    max_states: 2,
    branch: 2,
    max_depth: SearchBudget::DEFAULT.max_depth,
    rank_scan: SearchBudget::DEFAULT.rank_scan,
    scan: SearchBudget::DEFAULT.scan,
    max_millis: 6_000,
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

/// 展開図1つぶんの折り方を探し、通して確かめて、画面へ運べる形にする(作業27)。
///
/// 手順が1手も残らなかったときは `None` を返す。**折れない手を「折れる」として
/// 返さない**ため、確かめられた手だけを `steps` へ入れる。
fn plan_folds(
    skeleton: &Skeleton,
    packing: &Packing,
    cp: &CreasePattern,
    sites: &[LeafSite],
    paper: &Paper,
    paper_w: f64,
    paper_h: f64,
) -> Option<ProposalFoldPlan> {
    let mut document = Document::new(paper.clone());
    document.cp = cp.clone();
    let session = FoldSession::new(&document).ok()?;
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
    let outcome = search_to_completion(
        &session,
        &goal,
        GapWeights::DEFAULT,
        PLAN_BUDGET,
        CompletionTolerance::DEFAULT,
    );
    let order: Vec<usize> = outcome.steps.iter().map(|s| s.mv.id).collect();
    let verified = verify_search_completion(
        &session,
        &outcome,
        &goal,
        GapWeights::DEFAULT,
        PoseScan::DEFAULT,
        CompletionTolerance::DEFAULT,
    );
    let report = verified.report();
    // 通った手だけをもう一度たどって、展開図と手順を取り出す。
    let mut walk = session.clone();
    for step in &report.steps {
        let Some(Ok(mv)) = walk.check_move(step.id, PLAN_REBUILD_SCAN) else {
            break;
        };
        if walk.apply(&mv).is_err() {
            break;
        }
    }
    let folded = walk.document();
    if folded.sequence.is_empty() {
        return None;
    }
    let details = ProposalFoldPlanDetails {
        steps: folded.sequence.clone(),
        cp: folded.cp.clone(),
        planned: order.len(),
        checked: folded.sequence.len(),
    };
    Some(ProposalFoldPlan::from_verified(verified, details))
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
#[tauri::command(async)]
pub fn proposal_generate(
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
    with_fold_plan: bool,
) -> Result<Vec<ProposalCandidate>, String> {
    guard(AssertUnwindSafe(move || {
        skeleton.validate()?;
        let long = paper.width_mm.max(paper.height_mm);
        if !(long > 0.0 && long.is_finite()) {
            return Err("紙のサイズは正の値にしてください".to_string());
        }
        // CPの座標系は「紙の長辺=1.0」正規化(ori3_model::Document::new と同じ)
        let (w, h) = (paper.width_mm / long, paper.height_mm / long);
        let packings = pack(&skeleton, w, h, seed, PACK_STARTS);
        let mut out = Vec::new();
        let mut last_err = None;
        for p in &packings {
            match generate(&skeleton, p, w, h) {
                Ok(r) => {
                    // 折り方は展開図が決まってから探す。見つからなくても候補は返す
                    // (「止めずに警告する」。画面が「折り方は付いていません」と伝える)
                    let fold_plan = with_fold_plan
                        .then(|| plan_folds(&skeleton, p, &r.cp, &r.sites, &paper, w, h))
                        .flatten();
                    out.push(ProposalCandidate {
                        cp: r.cp,
                        scale: p.scale,
                        violations: r.violations,
                        warnings: r.warnings,
                        sites: r.sites,
                        fold_plan,
                    });
                }
                Err(e) => last_err = Some(e),
            }
        }
        if out.is_empty() {
            return Err(last_err.unwrap_or_else(|| {
                "この骨格を紙の上に配置できませんでした(角を減らすか短くしてみてください)"
                    .to_string()
            }));
        }
        Ok(out)
    }))
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
) -> Result<(), String> {
    guard(AssertUnwindSafe(move || {
        let doc = lock(&state).export_inputs(); // 複製のみ、即ロック解放
        // 先に全ページぶんを作り切る。途中の手順で失敗しても、その時点では
        // まだ1つもファイルを作っていないので中途半端な結果が残らない
        // 全ページを先にメモリへ作り、次に全て一時ファイルへ書く。途中で失敗しても
        // 既存の完成ファイルには触れない。
        let files = export_files(&doc, kind, options)?;
        write_export_files(Path::new(&path), &files)
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
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let opts = CpSvgOptions {
        include_aux: options.include_aux,
    };
    Ok(match kind {
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
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentStore, PLAN_BUDGET, ProposalFoldPlan, ProposalFoldPlanDetails,
        ProposalFoldPlanState, attach_replay, display_soft_settings, frame_surface_rank_order,
        guard, pose_motion_contact_options, pose_overlap_order, pose_result_is_finite,
        proposal_generate, record_finish_soft, recorded_soft_settings, stamp_saved_layer_order,
        usable_pose_surface_order,
    };
    use ori3_model::{
        DisplaySettings, Document, EdgeKind, Face3D, FaceId, FinishSoftSettings, FoldStep, Frame3D,
        Paper, SeqOp, TechniqueKind,
    };
    use ori3_propose::{
        CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, SearchBudget,
        Skeleton, SkeletonNode, search_to_completion,
    };
    use ori3_soft::SoftSettings;
    use std::collections::HashMap;
    use std::panic::AssertUnwindSafe;
    use std::path::{Path, PathBuf};

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
        assert!(with_plan >= 1, "折り方が付いた候補が1件も無い");
    }

    /// 折り方を付けない呼び方では、折り方の計算をまったく行わない。
    #[test]
    fn proposal_generate_without_a_fold_plan_leaves_it_empty() {
        let out = proposal_generate(star(6), A4ISH, 1, false).expect("候補が返るはず");
        assert!(!out.is_empty());
        assert!(out.iter().all(|c| c.fold_plan.is_none()));
    }

    /// 画面(`plan_folds`)が使う打ち切りは **6,000ms(6秒)** に固定されている。
    ///
    /// 根拠(最適化ありの実測、`PLAN_BUDGET` のコメントと
    /// `scratchpad/search-budget-report.md`): 折り鶴2.500秒・鳥の基本形0.825秒・
    /// やっこさん0.102秒のうち、いちばん重い折り鶴の約2.4倍。検査用の既定
    /// [`SearchBudget::MAX_MILLIS`](ori3_propose::SearchBudget::MAX_MILLIS)
    /// (240,000ms)とは別の値であることも合わせて固定する。
    #[test]
    fn plan_budget_caps_screen_wait_time_at_six_seconds() {
        assert_eq!(
            PLAN_BUDGET.max_millis,
            6_000,
            "画面用の時間打切りを根拠なく変えた"
        );
        assert_ne!(
            PLAN_BUDGET.max_millis,
            SearchBudget::MAX_MILLIS,
            "画面用の打切りが検査用の既定(240,000ms)のままでは、待ち時間が長すぎる"
        );
    }

    /// `plan_folds` と同じ形の探索(`PLAN_BUDGET` の状態数・分岐数)を、
    /// 時間だけ0msへ縮めて呼んでも、panicせず有限の最善を返すこと。
    ///
    /// 打ち切りは操作を止める仕組みではない(`CLAUDE.md` §8)。時間に当たっても
    /// そこまでの最善をそのまま返すことを、`plan_folds` が実際に使う
    /// `max_states = 2` / `branch = 2` の規模で確かめる。
    #[test]
    fn a_time_capped_plan_search_returns_a_finite_result_without_panicking() {
        let document = Document::new(A4ISH);
        let session = FoldSession::new(&document).expect("平らな正方形を読み込めない");
        let goal = FoldGoal {
            target: FinishTarget::default(),
            body: [0.5, 0.5],
            sites: Vec::new(),
        };
        let zero_time_budget = SearchBudget {
            max_millis: 0,
            ..PLAN_BUDGET
        };
        let outcome = search_to_completion(
            &session,
            &goal,
            GapWeights::DEFAULT,
            zero_time_budget,
            CompletionTolerance::DEFAULT,
        );
        assert!(
            outcome.best_gaps.all_finite(),
            "時間打切りで有限でない隔たりが返った"
        );
        assert!(
            outcome.best_score.is_finite(),
            "時間打切りで有限でない点数が返った"
        );
        assert!(outcome.start_gaps.all_finite());
        assert!(outcome.start_score.is_finite());
    }

    #[test]
    fn export_bytes_makes_svg_and_png() {
        use super::{ExportKind, ExportOptions, export_files};
        let doc = ori3_model::Document::new(A4ISH);
        let opts = ExportOptions {
            include_aux: true,
            png_long_side: 128,
        };
        let svg = export_files(&doc, ExportKind::CpSvg, opts).unwrap();
        assert_eq!(svg.len(), 1);
        assert!(svg[0].0.is_empty(), "1つのファイルなので番号は付かない");
        let text = String::from_utf8(svg[0].1.clone()).unwrap();
        assert!(text.contains("viewBox=\"0 0 150 150\""), "{text}");

        let png = export_files(&doc, ExportKind::CpPng, opts).unwrap();
        assert_eq!(&png[0].1[0..8], b"\x89PNG\r\n\x1a\n");

        // 点数が0など無理な指定は日本語のErr(パニックにしない)
        let bad = ExportOptions {
            include_aux: false,
            png_long_side: 0,
        };
        assert!(export_files(&doc, ExportKind::CpPng, bad).is_err());
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
}
