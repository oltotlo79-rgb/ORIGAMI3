//! IPCコマンド層: 各コマンドはDocumentStoreへ委譲するだけの薄いラッパー。
//! 全コマンドをpanic捕捉ラッパー`guard`で包み、アプリを落とさない(SYS-005)。
//! 全コマンドを`#[tauri::command(async)]`にしてスレッドプールで実行する
//! (同期fnはメインスレッド実行になり、validate等の計算でUIが引っかかるため)。
//!
//! 設計規約: ロック中に重い計算をしない(pose_solveなど他コマンドを待たせないため)。
//! ロック下ではstoreの状態更新と複製だけを行い、手順の再生や姿勢計算は
//! ロックを解放してから実行する(`view_command` / `pose_solve` / `sequence_replay`)。

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::autosave;
use crate::store::{
    DocumentStore, DocumentView, add_layer_order_warning,
    add_penetration_warning_for_intersections, attach_replay,
};
use ori3_export::{CpSvgOptions, cp_png, cp_svg, diagram_pdf, diagram_svg_pages};
use ori3_model::{CreasePattern, Driver, EdgeId, EditOp, Paper, SeqOp};
use ori3_propose::{Skeleton, generate, pack};
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
}

/// たわみの網を作る。指定が無い・切ってあるときは何もしない(従来どおりの動作)。
///
/// 設計規約: 重い計算なので必ずロックの外から呼ぶこと。
fn soft_mesh(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
    frame: &ori3_model::Frame3D,
    soft: Option<&SoftSettings>,
) -> Option<SoftMesh> {
    let settings = soft?;
    if !settings.enabled {
        return None;
    }
    Some(ori3_soft::relax(cp, faces, frame, settings))
}

/// 全値finiteの最良候補はそのまま表示し、数値が壊れた場合だけ直前形へ戻す。
fn fallback_nonfinite_pose(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
    warm: Option<&HashMap<EdgeId, f64>>,
    failed: ori3_rigid::SolveResult,
) -> ori3_rigid::SolveResult {
    let finite = failed.closure_rms.is_finite()
        && failed.angles.values().all(|angle| angle.is_finite())
        && failed.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        });
    if finite {
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

#[tauri::command(async)]
pub fn sequence_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: SeqOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(&state, || lock(&state).apply_seq(op))
    }))
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
) -> Result<PoseOutcome, String> {
    guard(AssertUnwindSafe(|| {
        let (cp, faces, stored_warm, overlap_enabled, penetration_enabled) =
            lock(&state).pose_inputs(); // 複製のみ、即ロック解放
        let preferred = preferred.unwrap_or_default();
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
        let motion = ori3_rigid::solve_motion(
            &cp,
            &faces,
            &hard,
            targets.as_ref(),
            warm,
            penetration_enabled,
        );
        let contact_detected = motion.contact_detected;
        let mut result = fallback_nonfinite_pose(&cp, &faces, warm, motion.result);
        // 手順を持たない角度操作では初期層順序(面ID順)を上下の契約にする。
        // 共有網頂点へ補正するので、折り目の接続は切れない。
        let mut order: Vec<ori3_model::FaceId> = faces.iter().map(|face| face.id).collect();
        order.sort_unstable();
        ori3_soft::prevent_overlap(
            &cp,
            &faces,
            &mut result.frame,
            &order,
            &order,
            0.5,
            &ori3_soft::OverlapSettings {
                enabled: overlap_enabled,
                ..Default::default()
            },
        );
        let intersections = ori3_rigid::self_intersection_pairs(&result.frame);
        let suspect_hinges = ori3_rigid::suspect_hinges_for_intersections(
            &cp,
            &faces,
            &intersections,
            &driver_hinges,
        );
        let _ = add_penetration_warning_for_intersections(
            &cp,
            &faces,
            &mut result.frame,
            false,
            &intersections,
        ); // SIM-007
        // たわみもロックの外で計算する(規約どおり)
        let mesh = soft_mesh(&cp, &faces, &result.frame, soft.as_ref());
        if result.closure_rms.is_finite() && result.angles.values().all(|angle| angle.is_finite()) {
            lock(&state).store_pose_angles(result.angles.clone()); // 短いロックで書き戻し
        }
        Ok(PoseOutcome {
            result,
            soft: mesh,
            suspect_hinges,
            contact_detected,
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
        let mut result = ori3_layers::replay_with_faces(&doc, &faces, up_to, t);
        let completed = !t.is_finite() || t >= 1.0;
        let mut penetration_warnings: Vec<&'static str> = Vec::new();
        if completed
            && let Some(warning) = add_layer_order_warning(&doc.cp, &faces, &mut result.frame)
        {
            penetration_warnings.push(warning);
        }
        let transition = result.layer_transition.clone();
        ori3_soft::prevent_overlap(
            &doc.cp,
            &faces,
            &mut result.frame,
            &transition.start,
            &transition.end,
            transition.progress,
            &ori3_soft::OverlapSettings {
                enabled: doc.display.overlap_prevention_enabled,
                ..Default::default()
            },
        );
        let intersections = ori3_rigid::self_intersection_pairs(&result.frame);
        let contact_detected = !intersections.is_empty();
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
        // たわみもロックの外で計算する(規約どおり)
        let mesh = soft_mesh(&doc.cp, &faces, &result.frame, soft.as_ref());
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
        })
    }))
}

/// 円充填のやり直し回数。多いほど良い配置に当たりやすいが時間もかかる。
/// 8回で12本の角でも数百ms以内に収まる(packing.rsの調整値と揃えてある)。
const PACK_STARTS: usize = 8;

/// 提案された展開図1つ分。`scale` は骨格の長さ1あたりが紙の何割になるか(大きいほど
/// 完成品が大きい)、`violations` は平坦に折りにくい頂点の数(0が理想)。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposalCandidate {
    pub cp: CreasePattern,
    pub scale: f64,
    pub violations: usize,
    pub warnings: Vec<String>,
}

/// 骨格から展開図の候補を作る(PRO-001/PRO-005、Task 3-4)。
/// 乱数の初期値違いで最大4つの候補を返し、どれを使うかは利用者が選ぶ。
///
/// 設計規約: ロック中に重い計算をしない。この処理は作品の状態を一切見ないので
/// storeのロックそのものを取らない(充填中も他のコマンドが普通に動く)。
#[tauri::command(async)]
pub fn proposal_generate(
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
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
                Ok(r) => out.push(ProposalCandidate {
                    cp: r.cp,
                    scale: p.scale,
                    violations: r.violations,
                    warnings: r.warnings,
                }),
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
    use super::{guard, proposal_generate};
    use ori3_model::Paper;
    use ori3_propose::{Skeleton, SkeletonNode};
    use std::panic::AssertUnwindSafe;

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
    fn proposal_generate_returns_candidates() {
        let out = proposal_generate(star(4), A4ISH, 7).expect("候補が返るはず");
        assert!(!out.is_empty() && out.len() <= 4, "件数={}", out.len());
        for c in &out {
            assert!(c.scale > 0.0, "scale={}", c.scale);
            // 輪郭4辺だけ、ということはない(折り線が引かれている)
            assert!(c.cp.edges.len() > 4, "辺数={}", c.cp.edges.len());
        }
    }

    #[test]
    fn proposal_generate_is_deterministic() {
        let a = proposal_generate(star(3), A4ISH, 42).unwrap();
        let b = proposal_generate(star(3), A4ISH, 42).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn proposal_generate_rejects_broken_skeleton() {
        // 角が1本もない(根だけ)骨格は骨格側の検査で日本語のErrになる
        let only_root = Skeleton {
            nodes: vec![SkeletonNode::new(0, None, 0.0)],
        };
        let err = proposal_generate(only_root, A4ISH, 1).unwrap_err();
        assert!(err.contains("角"), "err={err}");

        // 紙のサイズが0以下でもErr(パニックにしない)
        let bad_paper = Paper {
            width_mm: 0.0,
            height_mm: 0.0,
        };
        assert!(proposal_generate(star(2), bad_paper, 1).is_err());
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
        let doc = ori3_model::Document::new(A4ISH);
        let faces = ori3_cp::extract_faces(&doc.cp);
        let frame = ori3_layers::replay_with_faces(&doc, &faces, 0, 1.0).frame;

        assert!(soft_mesh(&doc.cp, &faces, &frame, None).is_none());
        let off = SoftSettings::default();
        assert!(!off.enabled, "たわみの既定はオフ");
        assert!(soft_mesh(&doc.cp, &faces, &frame, Some(&off)).is_none());

        let on = SoftSettings {
            enabled: true,
            ..SoftSettings::default()
        };
        let mesh = soft_mesh(&doc.cp, &faces, &frame, Some(&on)).expect("網が返るはず");
        assert!(!mesh.triangles.is_empty(), "三角形が無い");
        assert_eq!(mesh.triangles.len(), mesh.triangle_faces.len());
        assert_eq!(mesh.triangles.len(), mesh.triangle_layers.len());
        // 分割しているので、元の面(1枚=三角形2つ)より細かくなる
        assert!(mesh.triangles.len() > 2, "分割されていない");
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
}
