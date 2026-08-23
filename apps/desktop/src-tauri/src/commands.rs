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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
/// **どの大きさでも10回とも同じ結果**で、**`TimeCap` は1件も出ない**。
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
/// そこで**壁時計では切らず、計算量(`max_states` / `branch`)だけで切る**。
/// `max_millis` は「何かがおかしくて終わらない」ときのためだけに残す。
///
/// - 実測の最大は **13.851秒**(先端12本、10回の最大)
/// - その **2.2倍**にあたる **30,000ms(30秒)** を安全弁にする
/// - 実測は上限の **46.2%** で、`CLAUDE.md` §10.7.9 の「実測は上限の8割以内」を満たす
///
/// **この値に当たらないことは検査で見張る**
/// (`the_heaviest_proposal_never_hits_the_time_limit`)。将来また重くなったら、
/// 黙って答えが痩せるのではなく検査が落ちる。
///
/// 打ち切っても操作は止まらない: 打ち切った時点での最善を返し、画面には
/// 「途中に注意があります」(`ProposalFoldPlanState::Partial`、手数は別表示)と出る
/// (`apps/desktop/src/components/dialogs/ProposalWizard.tsx::foldPlanLabel`)。
const PLAN_BUDGET: SearchBudget = SearchBudget {
    max_states: 2,
    branch: 2,
    max_depth: SearchBudget::DEFAULT.max_depth,
    rank_scan: SearchBudget::DEFAULT.rank_scan,
    scan: SearchBudget::DEFAULT.scan,
    max_millis: 30_000,
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

/// 提案の計算が、候補いくつぶん終わったかを数える入れ物(1回の計算ぶん)。
///
/// 候補ごとの計算は互いに独立なので同時に走らせている([`generate_candidates`])。
/// **終わった件数だけ**を数え、どの候補が終わったかは持たない。
///
/// # なぜ数を直に置かず、「入れ物」を持ち回るか
///
/// 数の置き場をプロセスにひとつしか作らないと、**同時に走る2つの計算が
/// 同じ数を書き換える**。画面からは一度に1回しか計算できないが
/// (`apps/desktop/src/components/dialogs/ProposalWizard.tsx` の `disabled={busy}`)、
/// **検査は既定で同時に走る**ので、これが実際に起きた。
///
/// 実測(2026-08-24、`cargo test -p desktop --lib proposal` を12回):
/// `proposal_progress_counts_every_candidate` が **12回中9回**落ち、
/// 読めた数は `done = 5 / total = 4`、`done = 1 / total = 0`、
/// `done = 3 / total = 0` など、**1回の計算では有り得ない組**だった
/// (総数4件の計算で5件終わることはない)。
/// つまり数え落としではなく、別の計算の数が混ざっていた。
///
/// そこで数の置き場を引数で受け取れるようにした。画面の道すじ
/// ([`proposal_generate`] → [`PROPOSAL_PROGRESS`] → [`proposal_progress`])は
/// 今までと同じで、検査だけが**自分専用の入れ物**を渡す。
struct ProposalProgressCell {
    /// 計算が終わった候補の数。
    done: AtomicUsize,
    /// 計算する候補の総数。まだ始まっていなければ 0。
    total: AtomicUsize,
}

impl ProposalProgressCell {
    const fn new() -> Self {
        Self {
            done: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
        }
    }

    /// 計算を始めるときに数え直す。
    ///
    /// 先に終わった件数を0へ戻してから総数を入れる。逆にすると、
    /// 読み手が「前回の終わった件数 / 今回の総数」という有り得ない組を見ることがある。
    fn start(&self, total: usize) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    /// 候補1件ぶんの計算が終わった。
    fn finish_one(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }

    /// いまの数を読み取る。
    fn snapshot(&self) -> ProposalProgress {
        ProposalProgress {
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        }
    }
}

/// 画面の待ち表示が見る、プロセスにひとつだけの数。
///
/// ここを読み書きするのは**2箇所だけ**で、どちらも同じこの入れ物を指している。
/// 書く側が [`proposal_generate`]、読む側が [`proposal_progress`]。
static PROPOSAL_PROGRESS: ProposalProgressCell = ProposalProgressCell::new();

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

/// 検査のときだけ使う仕切り(製品の実行ファイルには入らない)。
///
/// 画面用の入れ物([`PROPOSAL_PROGRESS`])へ書く計算どうしは、同時に走ってよい
/// ので**読みの鍵**を取る。ただし「画面用の入れ物に本当に数が入るか」を見る検査
/// (`the_screen_reads_the_numbers_of_the_real_calculation`)だけは、
/// 計算と読み取りの間に他の計算が割り込むと数がすり替わるので、
/// **書きの鍵**を取ってその間だけ他の計算を止める。
#[cfg(test)]
static SCREEN_PROGRESS_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// 提案の計算の進み具合(画面の待ち表示に使う)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProposalProgress {
    /// 計算が終わった候補の数。
    pub done: usize,
    /// 計算する候補の数。まだ始まっていなければ 0。
    pub total: usize,
}

/// 提案の計算がどこまで進んだかを返す(作業27の待ち表示)。
///
/// 計算そのものは [`proposal_generate`] が別の場所で進めている。
/// この関数は数を読むだけで、作品の状態もロックも触らないので、
/// 計算中でもすぐ返る。
#[tauri::command]
#[must_use]
pub fn proposal_progress() -> ProposalProgress {
    PROPOSAL_PROGRESS.snapshot()
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
    budget: SearchBudget,
) -> Option<ProposalFoldPlan> {
    // 紙の長辺を1.0とした大きさ(`ori3_model::Document::new` と同じ正規化)。
    // 呼び出し側とまったく同じ式なので、受け取る代わりにここで出す。
    let long = paper.width_mm.max(paper.height_mm);
    let (paper_w, paper_h) = (paper.width_mm / long, paper.height_mm / long);
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
        budget,
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
        // 検査のときだけ、画面用の数を見る検査と重ならないようにする(読みの鍵なので、
        // 普通の計算どうしは今までどおり同時に走る)。製品の実行ファイルには入らない。
        #[cfg(test)]
        let _shared = SCREEN_PROGRESS_GATE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        generate_candidates(
            &skeleton,
            &paper,
            seed,
            with_fold_plan,
            PLAN_BUDGET,
            &PROPOSAL_PROGRESS,
        )
    }))
}

/// 候補を作る本体。**進み具合を書き込む先を引数で受け取る**。
///
/// 画面からは [`proposal_generate`] が [`PROPOSAL_PROGRESS`] を渡して呼ぶ。
/// 検査は自分専用の入れ物を渡し、同時に走る他の検査と数を取り合わないようにする
/// (理由と実測は [`ProposalProgressCell`] のコメント)。
fn generate_candidates(
    skeleton: &Skeleton,
    paper: &Paper,
    seed: u64,
    with_fold_plan: bool,
    budget: SearchBudget,
    progress: &ProposalProgressCell,
) -> Result<Vec<ProposalCandidate>, String> {
    // 前回の数字が残ったまま読まれないよう、いちばん先に0へ戻す。
    progress.start(0);
    skeleton.validate()?;
    let long = paper.width_mm.max(paper.height_mm);
    if !(long > 0.0 && long.is_finite()) {
        return Err("紙のサイズは正の値にしてください".to_string());
    }
    // CPの座標系は「紙の長辺=1.0」正規化(ori3_model::Document::new と同じ)
    let (w, h) = (paper.width_mm / long, paper.height_mm / long);
    let packings = pack(skeleton, w, h, seed, PACK_STARTS);
    // 候補は互いに独立なので、同時に計算する(理由と実測は `plan_folds` のコメント)。
    // 進み具合は画面が `proposal_progress` で読む。
    progress.start(packings.len());
    let planned: Vec<Result<ProposalCandidate, String>> = std::thread::scope(|scope| {
        let workers: Vec<_> = packings
            .iter()
            .map(|p| {
                scope.spawn(move || {
                    // 「終わった」を先に予約しておく。うまくいっても、失敗しても、
                    // 途中で落ちても、この札が捨てられるときにちょうど1つ数える。
                    let _ticket = CandidateTicket(progress);
                    generate(skeleton, p, w, h).map(|r| {
                        // 折り方は展開図が決まってから探す。見つからなくても候補は返す
                        // (「止めずに警告する」。画面が「折り方は付いていません」と伝える)
                        let fold_plan = with_fold_plan
                            .then(|| {
                                plan_folds(skeleton, p, &r.cp, &r.sites, paper, budget)
                            })
                            .flatten();
                        ProposalCandidate {
                            cp: r.cp,
                            scale: p.scale,
                            violations: r.violations,
                            warnings: r.warnings,
                            sites: r.sites,
                            fold_plan,
                        }
                    })
                })
            })
            .collect();
        workers
            .into_iter()
            // 1件でも内部で落ちたら、直列だったときと同じように外側の
            // `guard` まで持ち上げる。握りつぶして「候補が作れなかった」に
            // すり替えない。
            .map(|worker| worker.join().unwrap_or_else(|payload| std::panic::resume_unwind(payload)))
            .collect()
    });
    // 返す順番と中身は、直列で回していたときとまったく同じにする。
    let mut out = Vec::new();
    let mut last_err = None;
    for made in planned {
        match made {
            Ok(candidate) => out.push(candidate),
            Err(e) => last_err = Some(e),
        }
    }
    if out.is_empty() {
        return Err(last_err.unwrap_or_else(|| {
            "この骨格を紙の上に配置できませんでした(角を減らすか短くしてみてください)".to_string()
        }));
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
        CandidateTicket, DocumentStore, PACK_STARTS, PLAN_BUDGET, PROPOSAL_PROGRESS,
        ProposalCandidate, ProposalFoldPlan, ProposalFoldPlanDetails, ProposalFoldPlanState,
        ProposalProgress, ProposalProgressCell, SCREEN_PROGRESS_GATE, attach_replay,
        display_soft_settings, frame_surface_rank_order, generate_candidates, guard, plan_folds,
        pose_motion_contact_options, pose_overlap_order, pose_result_is_finite, proposal_generate,
        proposal_progress, record_finish_soft, recorded_soft_settings, stamp_saved_layer_order,
        usable_pose_surface_order,
    };
    use ori3_model::{
        DisplaySettings, Document, EdgeKind, Face3D, FaceId, FinishSoftSettings, FoldStep, Frame3D,
        Paper, SeqOp, TechniqueKind,
    };
    use ori3_propose::{
        CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, SearchBudget,
        SearchStop, Skeleton, SkeletonNode, TipSite, body_on_paper, generate, pack,
        search_to_completion,
    };
    use ori3_soft::SoftSettings;
    use std::collections::HashMap;
    use std::panic::AssertUnwindSafe;
    use std::path::{Path, PathBuf};


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
    const TIME_FREE_PLAN_BUDGET: SearchBudget = SearchBudget {
        max_millis: 3_600_000,
        ..PLAN_BUDGET
    };

    /// 候補を**1件ずつ順番に**計算する参照実装(`proposal_generate` の並列版と突き合わせる用)。
    ///
    /// `proposal_generate` を並列にする前の輪と同じ順序・同じ中身を作る。
    /// あわせて、候補ごとの探索の止まり方(`stop`)も集める。
    fn proposal_generate_one_by_one(
        skeleton: &Skeleton,
        paper: &Paper,
        seed: u64,
        budget: SearchBudget,
    ) -> (Vec<ProposalCandidate>, Vec<SearchStop>) {
        let long = paper.width_mm.max(paper.height_mm);
        let (w, h) = (paper.width_mm / long, paper.height_mm / long);
        let packings = pack(skeleton, w, h, seed, PACK_STARTS);
        let mut out = Vec::new();
        let mut stops = Vec::new();
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
                stops.push(
                    search_to_completion(
                        &session,
                        &goal,
                        GapWeights::DEFAULT,
                        budget,
                        CompletionTolerance::DEFAULT,
                    )
                    .stop,
                );
            }
            let fold_plan = plan_folds(skeleton, p, &r.cp, &r.sites, paper, budget);
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
    /// `PLAN_BUDGET.max_millis`(30,000ms)は**壁時計**の打ち切りである。
    /// 打ち切りに当たると、当たった側だけ答えが痩せる。つまり
    /// **答えが機械の速さと混み具合で変わる**。`CLAUDE.md` §10.7.7 が禁じている
    /// 「解の結果に期待値を結び付けたテスト」がまさにこの形で、実際に落ちた:
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
    /// `max_states` と `branch` は `PLAN_BUDGET` と**同じ 2・2 のまま**なので、
    /// 探索する計算量は画面と同じで、**主張の意味は変わらない**。
    /// `PLAN_BUDGET` そのものの値は変更していない。
    #[test]
    fn proposal_candidates_are_the_same_computed_together_or_one_by_one() {
        let skeleton = star(6);
        let progress = ProposalProgressCell::new();
        let together = generate_candidates(
            &skeleton,
            &A4ISH,
            1,
            true,
            TIME_FREE_PLAN_BUDGET,
            &progress,
        )
        .expect("候補が返るはず");
        let (one_by_one, _) =
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
        let progress = ProposalProgressCell::new();
        let out =
            generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &progress).expect("候補が返るはず");
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
        let progress = ProposalProgressCell::new();
        progress.start(1);
        let fell_over = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _ticket = CandidateTicket(&progress);
            panic!("候補の計算が落ちた");
        }));
        assert!(fell_over.is_err(), "前提: 落ちること");
        assert_eq!(
            progress.snapshot(),
            ProposalProgress { done: 1, total: 1 },
            "落ちた候補が『終わった』に数えられていない"
        );
    }

    /// 画面が読む数が、**本物の計算**が書いた数であること(道すじの確認)。
    ///
    /// 画面が呼ぶ入口(`proposal_generate`)は `PROPOSAL_PROGRESS` へ数を書き、
    /// 画面が読む入口(`proposal_progress`)は同じ `PROPOSAL_PROGRESS` を読む。
    /// この2箇所がつながっていないと、棒がいつまでも「準備中」のままになる。
    ///
    /// 計算と読み取りの間に別の検査の計算が割り込むと数がすり替わるので、
    /// その間だけ `SCREEN_PROGRESS_GATE` の**書きの鍵**で他の計算を止める
    /// (`proposal_generate` は読みの鍵なので、普段は同時に走れる)。
    #[test]
    fn the_screen_reads_the_numbers_of_the_real_calculation() {
        let _exclusive = SCREEN_PROGRESS_GATE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let out = generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &PROPOSAL_PROGRESS)
            .expect("候補が返るはず");
        let progress = proposal_progress();
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
    }

    /// 計算を始めた時点で、前回の数字が残っていないこと。
    ///
    /// 残っていると、画面が一瞬「4/4 件」と出してから0へ戻る。
    /// 骨格が不正で早く止まる場合でも0へ戻っていることを、同じ検査で見る。
    #[test]
    fn proposal_progress_is_cleared_before_the_next_calculation() {
        let progress = ProposalProgressCell::new();
        generate_candidates(&star(4), &A4ISH, 1, false, PLAN_BUDGET, &progress).expect("候補が返るはず");
        assert!(
            progress.snapshot().done > 0,
            "前提: 1回目で数字が入っていること"
        );
        let only_root = Skeleton {
            nodes: vec![SkeletonNode::new(0, None, 0.0)],
        };
        generate_candidates(&only_root, &A4ISH, 1, false, PLAN_BUDGET, &progress)
            .expect_err("角の無い骨格は作れない");
        let cleared = progress.snapshot();
        assert_eq!(
            (cleared.done, cleared.total),
            (0, 0),
            "次の計算の前に数字が0へ戻っていない: {cleared:?}"
        );
    }

    /// 候補ごとの探索の止まり方だけを集める(21姿勢の確認と手順の組み直しはしない)。
    ///
    /// 時間の打ち切りに当たるかどうかは**探索の中で決まる**ので、
    /// 見張るのにその先まで走らせる必要はない。恒久の検査を軽くするために分けてある。
    fn proposal_stops_together(
        skeleton: &Skeleton,
        paper: &Paper,
        seed: u64,
        budget: SearchBudget,
    ) -> Vec<SearchStop> {
        let long = paper.width_mm.max(paper.height_mm);
        let (w, h) = (paper.width_mm / long, paper.height_mm / long);
        let packings = pack(skeleton, w, h, seed, PACK_STARTS);
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
                        Some(
                            search_to_completion(
                                &session,
                                &goal,
                                GapWeights::DEFAULT,
                                budget,
                                CompletionTolerance::DEFAULT,
                            )
                            .stop,
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

    /// **いちばん重い骨格でも、時間の打ち切りに当たらないこと。**
    ///
    /// # なぜこの検査が要るか
    ///
    /// 時間の打ち切り([`PLAN_BUDGET`] の `max_millis`)は**壁時計**なので、
    /// 当たると**同じ入力でも機械の速さと混み具合で答えが変わる**。
    /// 2026-08-23より前は 6,000ms で切っており、先端12本では候補4件のうち
    /// **2件が実際に当たっていた**。同じ計算をもう一度しただけで当たる件数が
    /// 2件→1件と変わることも実測しており、`CLAUDE.md` §10.7.7 が禁じる
    /// 「解の結果を計算機に依存させる」形になっていた。
    ///
    /// いまは**計算量だけで切り**、時間の上限は安全弁として残している。
    /// **その安全弁に当たっていないこと**を、ここで見張る。
    /// 将来また重くなったら、黙って答えが痩せるのではなく**この検査が落ちる**。
    ///
    /// 骨格は先端12本([`ori3_propose::MAX_LEAVES`] の上限＝いちばん重い)。
    ///
    /// ## この主張は最適化ありでしか成り立たない(2026-08-24追記)
    ///
    /// 時間の打ち切り(`PLAN_BUDGET.max_millis`)は壁時計であり、
    /// 最適化なしは最適化ありより16.8〜20.5倍遅い(`store.rs` の
    /// `checked_head_tail_four_legs_proposal_is_consumed_and_one_undo_restores_the_work`
    /// で実測済み)。実際に測ると、最適化なし(`cargo test -p desktop --lib`)では
    /// 先端12本の4候補**すべて**が`TimeCap`に当たる
    /// (`[TimeCap, TimeCap, TimeCap, TimeCap]`、2026-08-24実測)。
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
        let stops = proposal_stops_together(&star(12), &A4ISH, 1, PLAN_BUDGET);
        assert!(!stops.is_empty(), "候補が1件も作られていない");
        let hit: Vec<_> = stops
            .iter()
            .filter(|stop| **stop == SearchStop::TimeCap)
            .collect();
        assert!(
            hit.is_empty(),
            "時間の打ち切りに当たった候補が {}件ある。答えが機械の混み具合で変わる状態に戻っている: {stops:?}",
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
        // **打ち切りに当たっている**(`SearchStop::TimeCap`)。1手が先に見つかっていた
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
    /// [`SearchBudget::MAX_MILLIS`](ori3_propose::SearchBudget::MAX_MILLIS)
    /// とは別の値であることも合わせて固定する。
    ///
    /// **値をぴったり固定したままにしてある**ので、次に根拠なく変えれば落ちる。
    #[test]
    fn plan_budget_keeps_the_screen_time_limit_as_a_safety_valve() {
        assert_eq!(
            PLAN_BUDGET.max_millis,
            30_000,
            "画面用の時間打切りを根拠なく変えた"
        );
        assert_ne!(
            PLAN_BUDGET.max_millis,
            SearchBudget::MAX_MILLIS,
            "画面用の打切りが検査用の既定のままでは、待ち時間の見積もりが立たない"
        );
        assert_eq!(
            (PLAN_BUDGET.max_states, PLAN_BUDGET.branch),
            (2, 2),
            "計算量の上限を根拠なく変えた(先端12本で 2/2 は8手、1/2 は4手へ半減する)"
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
