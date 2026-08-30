//! 自動保存とクラッシュ復旧(SYS-003)。
//!
//! 30秒ごとに、未保存の変更があるときだけ作業中の内容をアプリデータ配下の
//! 専用ファイルへ書き出す。実行中の作業は常に1件の専用枠へ書き、異常終了した
//! 作業は次回起動時に持ち越し候補へ移す。持ち越し候補は利用者が破棄するか、
//! 復元後に明示保存するまで消さない。
//!
//! 設計規約: ロックは複製を取る一瞬だけ。JSON化とファイル書き出しはロックの外。
//! `DocumentStore::save` は使わない(保存先パスと未保存フラグを書き換えるため)。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ori3_model::SavedDocument;

use crate::store::{DocumentStore, DocumentView, parse_document, write_atomic};

/// 自動保存の間隔(SYS-003: 30秒ごと)
pub const INTERVAL: Duration = Duration::from_secs(30);

/// 保存先が決まっていない作品の自動保存ファイル名(アプリデータ配下)
const UNTITLED_FILE: &str = "無題.ori3.autosave";

/// 自動保存の書き出し先を控えておく目印ファイル(次の起動時に読む)
const MARKER_FILE: &str = "autosave-location.txt";

/// 自動保存ファイルにつける拡張子
const SUFFIX: &str = ".autosave";

/// 複数候補の索引。旧来の目印は初回起動時だけここへ移行する。
const INDEX_FILE: &str = "autosave-index.json";
/// 今回起動中の作業だけが上書きする専用ファイル。
const CURRENT_FILE: &str = "autosave-current.ori3";
/// 異常終了後に持ち越す候補の保存先。
const CANDIDATES_DIR: &str = "autosave-recovery";
const INDEX_VERSION: u8 = 1;

/// 復元の案内に必要な情報(フロントの復旧ダイアログが使う)
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryInfo {
    /// 持ち越し候補を選ぶための識別子。現在の作業枠は持ち越し候補ではないためNone。
    pub candidate_id: Option<u64>,
    /// 自動保存ファイルの場所
    pub autosave_path: String,
    /// 元の保存先(無題だったならNone)
    pub document_path: Option<String>,
    /// 最後に自動保存した時刻(1970年からのミリ秒)。分からなければNone
    pub saved_at_ms: Option<u64>,
    /// 保存した折り手順の数。
    pub step_count: usize,
}

/// 起動時に復旧画面へ出す、持ち越し候補の一覧と超過件数。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryChoices {
    pub choices: Vec<RecoveryInfo>,
    pub overflow_count: usize,
}

/// 現在起動中の作業を控える枠の索引情報。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ActiveSnapshot {
    document_path: Option<String>,
    saved_at_ms: u64,
    step_count: usize,
    /// 旧来の単一候補APIの互換用。通常の起動経路では、起動前に持ち越しへ移る。
    visible_for_legacy_check: bool,
    /// 復元した候補。明示保存まで元の候補を残し、次回の異常終了ではこの候補へ戻す。
    #[serde(default)]
    source_candidate_id: Option<u64>,
}

/// 利用者が決めるまで残す候補の索引情報。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct StoredCandidate {
    id: u64,
    document_path: Option<String>,
    saved_at_ms: u64,
    step_count: usize,
}

/// payloadファイルと候補の説明を結ぶ索引。payloadは常にアプリデータの配下だけに置く。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct AutosaveIndex {
    version: u8,
    next_candidate_id: u64,
    active: Option<ActiveSnapshot>,
    carried: Vec<StoredCandidate>,
}

impl Default for AutosaveIndex {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            next_candidate_id: 1,
            active: None,
            carried: Vec::new(),
        }
    }
}

/// 自動保存ファイルの場所。保存先があれば `<保存先>.autosave`、
/// 無題ならアプリデータ配下の固定名。
fn target_path(doc_path: Option<&Path>, app_data: &Path) -> PathBuf {
    match doc_path {
        Some(p) => {
            let mut name = p.as_os_str().to_os_string();
            name.push(SUFFIX);
            PathBuf::from(name)
        }
        None => app_data.join(UNTITLED_FILE),
    }
}

fn marker_path(app_data: &Path) -> PathBuf {
    app_data.join(MARKER_FILE)
}

fn index_path(app_data: &Path) -> PathBuf {
    app_data.join(INDEX_FILE)
}

fn active_path(app_data: &Path) -> PathBuf {
    app_data.join(CURRENT_FILE)
}

fn candidate_path(app_data: &Path, id: u64) -> PathBuf {
    app_data.join(CANDIDATES_DIR).join(format!("{id}.ori3"))
}

fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("自動保存の時刻を取得できませんでした: {e}"))
        .and_then(|duration| {
            u64::try_from(duration.as_millis())
                .map_err(|_| "自動保存の時刻が大きすぎます".to_owned())
        })
}

fn read_index(app_data: &Path) -> Result<Option<AutosaveIndex>, String> {
    let path = index_path(app_data);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("自動保存の索引を読み込めませんでした: {error}")),
    };
    let index: AutosaveIndex = serde_json::from_str(&text)
        .map_err(|e| format!("自動保存の索引を読み込めませんでした: {e}"))?;
    if index.version != INDEX_VERSION {
        return Err("対応していない自動保存の索引です".to_owned());
    }
    Ok(Some(index))
}

fn write_index(app_data: &Path, index: &AutosaveIndex) -> Result<(), String> {
    let text = serde_json::to_string_pretty(index)
        .map_err(|e| format!("自動保存の索引を作成できませんでした: {e}"))?;
    write_atomic(index_path(app_data).as_path(), text.as_bytes())
        .map_err(|e| format!("自動保存の索引を書き込めませんでした: {e}"))
}

fn fresh_candidate_id(index: &mut AutosaveIndex) -> u64 {
    let id = index.next_candidate_id;
    index.next_candidate_id = index.next_candidate_id.saturating_add(1);
    id
}

fn active_info(app_data: &Path, active: &ActiveSnapshot) -> RecoveryInfo {
    RecoveryInfo {
        candidate_id: None,
        autosave_path: active_path(app_data).to_string_lossy().into_owned(),
        document_path: active.document_path.clone(),
        saved_at_ms: Some(active.saved_at_ms),
        step_count: active.step_count,
    }
}

fn candidate_info(app_data: &Path, candidate: &StoredCandidate) -> RecoveryInfo {
    RecoveryInfo {
        candidate_id: Some(candidate.id),
        autosave_path: candidate_path(app_data, candidate.id)
            .to_string_lossy()
            .into_owned(),
        document_path: candidate.document_path.clone(),
        saved_at_ms: Some(candidate.saved_at_ms),
        step_count: candidate.step_count,
    }
}

/// パスが実在する場合に、シンボリックリンクや`..`を解決して同じ場所か比べる。
fn same_existing_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// アプリデータの外にあるファイルを、改変された目印から消さないための検査。
///
/// 無題の自動保存はアプリデータ配下だけを許し、名前付き作品は現在の保存先に
/// `.autosave` を足した**その1ファイル**だけを許す。現在の作品が無い起動直後は
/// アプリデータ外の自動保存を読んで復旧を提案しても、削除はしない。
fn valid_autosave_path(autosave: &Path, app_data: &Path, current_document: Option<&Path>) -> bool {
    if autosave.extension().and_then(|e| e.to_str()) != Some("autosave") {
        return false;
    }
    let in_app_data = match (autosave.canonicalize(), app_data.canonicalize()) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => false,
    };
    if in_app_data {
        return true;
    }
    if let Some(document) = current_document {
        return same_existing_path(autosave, &target_path(Some(document), app_data));
    }
    false
}

/// 目印を読み、削除・復旧してよい自動保存だけを返す。
fn recorded_autosave_path(
    app_data: &Path,
    current_document: Option<&Path>,
    for_recovery: bool,
) -> Option<PathBuf> {
    let recorded = std::fs::read_to_string(marker_path(app_data)).ok()?;
    let autosave = PathBuf::from(recorded.trim());
    let recovery_path = for_recovery
        && current_document.is_none()
        && document_path_of(&autosave).is_some_and(|document| {
            document.is_file()
                && same_existing_path(&autosave, &target_path(Some(&document), app_data))
        });
    if valid_autosave_path(&autosave, app_data, current_document) || recovery_path {
        Some(autosave)
    } else {
        eprintln!(
            "自動保存の目印に許可されない場所が指定されているため無視しました: {}",
            autosave.display()
        );
        None
    }
}

/// 自動保存ファイルの場所から元の保存先を割り出す(無題ならNone)。
fn document_path_of(autosave: &Path) -> Option<PathBuf> {
    let text = autosave.to_str()?;
    let original = text.strip_suffix(SUFFIX)?;
    if autosave.file_name()? == UNTITLED_FILE {
        return None;
    }
    Some(PathBuf::from(original))
}

/// 旧来の1件だけの自動保存を、初回の複数候補起動時に持ち越し候補へ移す。
/// 元のpayloadと目印は削除せず、索引への書き込みが失敗しても前の復旧手段を失わない。
fn import_legacy_candidate(app_data: &Path, index: &mut AutosaveIndex) -> Result<bool, String> {
    let Some(legacy_path) = recorded_autosave_path(app_data, None, true) else {
        return Ok(false);
    };
    let text = std::fs::read_to_string(&legacy_path)
        .map_err(|e| format!("以前の自動保存を読み込めませんでした: {e}"))?;
    let saved = parse_document(&text)?;
    let id = fresh_candidate_id(index);
    std::fs::create_dir_all(app_data.join(CANDIDATES_DIR))
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(candidate_path(app_data, id).as_path(), text.as_bytes())
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    let saved_at_ms = std::fs::metadata(&legacy_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(now_ms()?);
    index.carried.push(StoredCandidate {
        id,
        document_path: document_path_of(&legacy_path)
            .map(|path| path.to_string_lossy().into_owned()),
        saved_at_ms,
        step_count: saved.document.sequence.len(),
    });
    Ok(true)
}

/// 起動ごとに、前回動いていた作業枠を利用者が選べる持ち越し候補へ移す。
/// 持ち越し件数が3件を超えても削除しない。今の作業枠は常に空けておく。
fn prepare_session(app_data: &Path) -> Result<(), String> {
    std::fs::create_dir_all(app_data).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    let mut index = match read_index(app_data)? {
        Some(index) => index,
        None => {
            let mut index = AutosaveIndex::default();
            if !import_legacy_candidate(app_data, &mut index)? {
                return Ok(());
            }
            index
        }
    };
    let Some(active) = index.active.take() else {
        write_index(app_data, &index)?;
        return Ok(());
    };
    let payload = std::fs::read(active_path(app_data))
        .map_err(|e| format!("前回の自動保存を読み込めませんでした: {e}"))?;
    if let Some(source_id) = active.source_candidate_id
        && let Some(candidate) = index
            .carried
            .iter_mut()
            .find(|candidate| candidate.id == source_id)
    {
        write_atomic(candidate_path(app_data, source_id).as_path(), &payload)
            .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
        candidate.document_path = active.document_path.clone();
        candidate.saved_at_ms = active.saved_at_ms;
        candidate.step_count = active.step_count;
        write_index(app_data, &index)?;
        std::fs::remove_file(active_path(app_data)).ok();
        return Ok(());
    }
    let id = fresh_candidate_id(&mut index);
    std::fs::create_dir_all(app_data.join(CANDIDATES_DIR))
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(candidate_path(app_data, id).as_path(), &payload)
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    index.carried.push(StoredCandidate {
        id,
        document_path: active.document_path,
        saved_at_ms: active.saved_at_ms,
        step_count: active.step_count,
    });
    write_index(app_data, &index)?;
    std::fs::remove_file(active_path(app_data)).ok();
    Ok(())
}

/// 復旧候補をあとで選ぶ前に、現在の作業枠があれば持ち越し候補へ写す。
/// 「あとで確認する」の後に編集した内容を、過去の候補を復元する操作で失わないため。
fn preserve_active_as_candidate(app_data: &Path, index: &mut AutosaveIndex) -> Result<(), String> {
    let Some(active) = index.active.clone() else {
        return Ok(());
    };
    let payload = std::fs::read(active_path(app_data))
        .map_err(|e| format!("今の作業の控えを読み込めませんでした: {e}"))?;
    if let Some(source_id) = active.source_candidate_id
        && let Some(candidate) = index
            .carried
            .iter_mut()
            .find(|candidate| candidate.id == source_id)
    {
        write_atomic(candidate_path(app_data, source_id).as_path(), &payload)
            .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
        candidate.document_path = active.document_path.clone();
        candidate.saved_at_ms = active.saved_at_ms;
        candidate.step_count = active.step_count;
        return Ok(());
    }
    let id = fresh_candidate_id(index);
    std::fs::create_dir_all(app_data.join(CANDIDATES_DIR))
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(candidate_path(app_data, id).as_path(), &payload)
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    index.carried.push(StoredCandidate {
        id,
        document_path: active.document_path.clone(),
        saved_at_ms: active.saved_at_ms,
        step_count: active.step_count,
    });
    Ok(())
}

/// 自動保存を1回行う。未保存の変更が無ければ何もせずfalseを返す。
/// ロックは複製を取る間だけ持ち、JSON化と書き出しはロックの外で行う。
pub fn run_once(store: &Mutex<DocumentStore>, app_data: &Path) -> Result<bool, String> {
    // 過去のpanicで毒化されていても中身を取り出して続ける(commands::lockと同じ規約)
    let snapshot = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .autosave_snapshot();
    let Some((doc_path, doc)) = snapshot else {
        return Ok(false); // ここでロックは解放済み
    };
    write_snapshot(&doc, doc_path.as_deref(), app_data)?;
    Ok(true)
}

/// 複製した作品を今回起動中だけの専用枠へ書く。持ち越し候補は変更しないため、
/// 利用者が復旧を決める前に別作品を編集しても以前の候補は上書きされない。
fn write_snapshot(
    doc: &SavedDocument,
    doc_path: Option<&Path>,
    app_data: &Path,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(doc)
        .map_err(|e| format!("自動保存データの作成に失敗しました: {e}"))?;
    std::fs::create_dir_all(app_data).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(active_path(app_data).as_path(), json.as_bytes())
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    let mut index = read_index(app_data)?.unwrap_or_default();
    let source_candidate_id = index
        .active
        .as_ref()
        .and_then(|active| active.source_candidate_id);
    index.active = Some(ActiveSnapshot {
        document_path: doc_path.map(|path| path.to_string_lossy().into_owned()),
        saved_at_ms: now_ms()?,
        step_count: doc.document.sequence.len(),
        visible_for_legacy_check: true,
        source_candidate_id,
    });
    write_index(app_data, &index)?;
    Ok(())
}

/// 持ち越している候補をすべて返す。現在の作業枠は起動ごとにここへ移るため、
/// 起動時の復旧画面が同じ作業を二重に提案することはない。
pub fn check_all(app_data: &Path) -> Result<RecoveryChoices, String> {
    let Some(index) = read_index(app_data)? else {
        return Ok(RecoveryChoices {
            choices: Vec::new(),
            overflow_count: 0,
        });
    };
    let active_source = index
        .active
        .as_ref()
        .and_then(|active| active.source_candidate_id);
    let choices = index
        .carried
        .iter()
        .filter(|candidate| Some(candidate.id) != active_source)
        .filter(|candidate| candidate_path(app_data, candidate.id).is_file())
        .map(|candidate| candidate_info(app_data, candidate))
        .collect::<Vec<_>>();
    Ok(RecoveryChoices {
        overflow_count: choices.len().saturating_sub(3),
        choices,
    })
}

/// 前回の自動保存が残っていれば、その情報を返す(起動時の復旧確認)。
/// 正常終了・明示保存のたびに消しているので、残っていれば異常終了とみなせる。
pub fn check(app_data: &Path) -> Option<RecoveryInfo> {
    if let Some(candidate) = check_all(app_data).ok()?.choices.into_iter().next() {
        return Some(candidate);
    }
    let index = read_index(app_data).ok().flatten()?;
    index
        .active
        .as_ref()
        .filter(|active| active.visible_for_legacy_check && active_path(app_data).is_file())
        .map(|active| active_info(app_data, active))
}

/// 利用者が明示的に選んだ持ち越し候補だけを破棄する。
/// `None`は旧単一候補APIとの互換用で、最初の候補だけを指す。
pub fn discard_candidate(app_data: &Path, candidate_id: Option<u64>) -> Result<bool, String> {
    let Some(mut index) = read_index(app_data)? else {
        return Ok(false);
    };
    let id = candidate_id.or_else(|| index.carried.first().map(|candidate| candidate.id));
    let Some(id) = id else {
        return Ok(false);
    };
    let Some(position) = index
        .carried
        .iter()
        .position(|candidate| candidate.id == id)
    else {
        return Ok(false);
    };
    index.carried.remove(position);
    // 先に索引を確定する。失敗時はpayloadも候補もそのままなので、利用者が選び直せる。
    write_index(app_data, &index)?;
    std::fs::remove_file(candidate_path(app_data, id)).ok();
    Ok(true)
}

/// 利用者が選んだ候補を現在の作業枠へ写して復元する。
/// 元の候補は明示保存まで残すため、復元直後に再び異常終了しても内容を失わない。
pub fn restore_candidate(
    store: &Mutex<DocumentStore>,
    app_data: &Path,
    candidate_id: Option<u64>,
) -> Result<Option<DocumentView>, String> {
    let Some(mut index) = read_index(app_data)? else {
        return Ok(None);
    };
    let id = candidate_id.or_else(|| index.carried.first().map(|candidate| candidate.id));
    let Some(id) = id else {
        return Ok(None);
    };
    let Some(position) = index
        .carried
        .iter()
        .position(|candidate| candidate.id == id)
    else {
        return Ok(None);
    };
    let candidate = index.carried[position].clone();
    let payload_path = candidate_path(app_data, id);
    let text = std::fs::read_to_string(&payload_path)
        .map_err(|e| format!("作業中だった内容を読み込めませんでした: {e}"))?;
    let doc = parse_document(&text)?;

    // 画面の状態を入れ替える前に、選んだ内容を現行作業枠へ確定する。
    preserve_active_as_candidate(app_data, &mut index)?;
    write_atomic(active_path(app_data).as_path(), text.as_bytes())
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    index.active = Some(ActiveSnapshot {
        document_path: candidate.document_path.clone(),
        saved_at_ms: now_ms()?,
        step_count: candidate.step_count,
        visible_for_legacy_check: false,
        source_candidate_id: Some(id),
    });
    write_index(app_data, &index)?;

    let path = candidate.document_path.map(PathBuf::from);
    let view = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .restore(doc, path);
    Ok(Some(view))
}

/// 今回起動中の作業枠だけを片付ける。持ち越し候補は、明示保存に成功した
/// 復元元だけを必要に応じて対象にし、それ以外は利用者が選ぶまで変更しない。
/// 索引が無い旧形式だけは、従来の目印と自動保存ファイルを片付ける。
fn discard_current_snapshot(
    app_data: &Path,
    current_document: Option<&Path>,
    discard_restored_source: bool,
) {
    if let Ok(Some(mut index)) = read_index(app_data) {
        let Some(active) = index.active.take() else {
            // 復旧画面で何も選んでいないときは持ち越し候補だけがある。
            // 正常終了でも別作品の明示保存でも、ここから候補を選んで消してはいけない。
            return;
        };
        let restored_source = discard_restored_source
            .then_some(active.source_candidate_id)
            .flatten();
        if let Some(id) = restored_source {
            index.carried.retain(|candidate| candidate.id != id);
        }
        write_index(app_data, &index).ok();
        std::fs::remove_file(active_path(app_data)).ok();
        if let Some(id) = restored_source {
            std::fs::remove_file(candidate_path(app_data, id)).ok();
        }
        return;
    }
    if let Some(autosave) = recorded_autosave_path(app_data, current_document, false) {
        std::fs::remove_file(autosave).ok();
    }
    std::fs::remove_file(marker_path(app_data)).ok();
}

/// 明示保存に成功した今回の作業枠を片付ける。復旧候補を開いていた場合だけ、
/// その復元元も保存済みとして消す。他の持ち越し候補には触れない。
/// 消せなくても保存成功自体は止めない。
pub fn discard_after_save(app_data: &Path, current_document: Option<&Path>) {
    discard_current_snapshot(app_data, current_document, true);
}

/// 正常終了時だけ自動保存を片付ける。未保存なら次回の復旧に残す。
pub fn discard_if_clean(store: &Mutex<DocumentStore>, app_data: &Path) {
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_dirty() {
        return;
    }
    let path = guard.current_path();
    drop(guard);
    // 正常終了は今回の作業枠だけを対象にする。復元元を含む持ち越し候補は
    // 利用者が破棄するか、その内容を明示保存するまで残す。
    discard_current_snapshot(app_data, path.as_deref(), false);
}

/// 自動保存の内容を読み込んで現在の作品にする。残っていなければNone。
/// 読み込み・JSON解釈はロックの外で行い、状態の入れ替えだけをロック下で行う。
pub fn restore(
    store: &Mutex<DocumentStore>,
    app_data: &Path,
) -> Result<Option<DocumentView>, String> {
    let Some(info) = check(app_data) else {
        return Ok(None);
    };
    if info.candidate_id.is_some() {
        return restore_candidate(store, app_data, info.candidate_id);
    }
    let text = std::fs::read_to_string(&info.autosave_path)
        .map_err(|e| format!("作業中だった内容を読み込めませんでした: {e}"))?;
    let doc = parse_document(&text)?;
    let step_count = doc.document.sequence.len();
    let path = info.document_path.clone().map(PathBuf::from);
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    let view = guard.restore(doc, path);
    drop(guard);
    // 復元した内容は今回の作業枠へ移す。明示保存に成功するまで内容は残し、
    // 同じ候補だけを復旧画面から繰り返し出さない。
    let mut index = read_index(app_data)?.unwrap_or_default();
    write_atomic(active_path(app_data).as_path(), text.as_bytes())
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    if let Some(id) = info.candidate_id {
        index.carried.retain(|candidate| candidate.id != id);
        std::fs::remove_file(candidate_path(app_data, id)).ok();
    }
    index.active = Some(ActiveSnapshot {
        document_path: info.document_path,
        saved_at_ms: now_ms()?,
        step_count,
        visible_for_legacy_check: false,
        source_candidate_id: None,
    });
    write_index(app_data, &index)?;
    Ok(Some(view))
}

/// アプリデータディレクトリ(無題の自動保存と目印ファイルの置き場)。
///
/// `ORI3_TEST_APP_DATA_DIR` は起動時のautosave/recovery検査だけが使う隔離入口。
/// 未設定または空文字なら通常のTauri app-data directoryを使うため、利用者が使う
/// 保存先は変わらない。
fn test_app_data_dir_override(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|path| !path.is_empty()).map(PathBuf::from)
}

pub fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Some(dir) = test_app_data_dir_override(std::env::var_os("ORI3_TEST_APP_DATA_DIR")) {
        return Ok(dir);
    }
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .map_err(|e| format!("保存場所を用意できませんでした: {e}"))
}

/// 自動保存のバックグラウンドスレッドを起こす(30秒ごと)。
/// 書き出しに失敗しても止めない(次の回で書き直せるため。「止めずに警告」原則)。
pub fn spawn(app: tauri::AppHandle) {
    let Ok(app_data) = app_data_dir(&app) else {
        return;
    };
    if let Err(error) = prepare_session(&app_data) {
        eprintln!("{error}");
    }
    std::thread::spawn(move || {
        use tauri::Manager;
        loop {
            std::thread::sleep(INTERVAL);
            let store = app.state::<Mutex<DocumentStore>>();
            if let Err(e) = run_once(&store, &app_data) {
                eprintln!("{e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_model::{EdgeKind, EditOp};

    /// テスト専用の作業ディレクトリ(アプリデータの代わり)
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ori3_autosave_{}_{tag}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store_with_edit() -> Mutex<DocumentStore> {
        let mut store = DocumentStore::default();
        store
            .apply_edit(EditOp::AddSegment {
                a: [0.0, 0.0],
                b: [1.0, 1.0],
                kind: EdgeKind::Mountain,
            })
            .unwrap();
        Mutex::new(store)
    }

    /// 自動保存は保存先パスと未保存フラグを変えない(saveの流用禁止の核心)。
    #[test]
    fn autosave_keeps_path_and_dirty_flag() {
        let dir = temp_dir("keep");
        let doc_path = dir.join("作品.ori3");
        let store = store_with_edit();
        store.lock().unwrap().save(Some(&doc_path)).unwrap();
        // 保存後にもう一度編集して未保存の状態を作る
        store
            .lock()
            .unwrap()
            .apply_edit(EditOp::AddSegment {
                a: [1.0, 0.0],
                b: [0.0, 1.0],
                kind: EdgeKind::Valley,
            })
            .unwrap();

        assert!(run_once(&store, &dir).unwrap(), "未保存なので書き出す");
        let s = store.lock().unwrap();
        assert_eq!(
            s.current_path(),
            Some(doc_path.clone()),
            "保存先は乗っ取らない"
        );
        assert!(s.is_dirty(), "未保存の印は消さない");
        drop(s);
        // 自動保存はアプリデータ内の現行作業枠へだけ書き、元ファイルは上書きしない。
        let autosave = active_path(&dir);
        assert!(autosave.is_file());
        let saved = std::fs::read_to_string(&doc_path).unwrap();
        assert!(!saved.contains("Valley"), "明示保存の内容は変わらない");
    }

    /// 未保存の変更が無ければ書かない。無題の作品はアプリデータ配下へ書く。
    #[test]
    fn autosave_skips_clean_document_and_writes_untitled_to_app_data() {
        let dir = temp_dir("clean");
        let store = Mutex::new(DocumentStore::default());
        assert!(!run_once(&store, &dir).unwrap(), "変更が無ければ書かない");
        assert!(check(&dir).is_none());

        store
            .lock()
            .unwrap()
            .apply_edit(EditOp::AddSegment {
                a: [0.0, 0.0],
                b: [1.0, 1.0],
                kind: EdgeKind::Mountain,
            })
            .unwrap();
        assert!(run_once(&store, &dir).unwrap());
        let info = check(&dir).expect("自動保存が残っている");
        assert_eq!(info.autosave_path, active_path(&dir).to_string_lossy());
        assert_eq!(info.document_path, None, "無題なので元ファイルは無い");
        assert!(info.saved_at_ms.is_some());
    }

    /// 復元すると内容が一致し、元の保存先も引き継ぐ。提案は繰り返さない。
    #[test]
    fn restore_recovers_the_same_document() {
        let dir = temp_dir("restore");
        let doc_path = dir.join("作品.ori3");
        let store = store_with_edit();
        store.lock().unwrap().save(Some(&doc_path)).unwrap();
        store
            .lock()
            .unwrap()
            .apply_edit(EditOp::AddSegment {
                a: [1.0, 0.0],
                b: [0.0, 1.0],
                kind: EdgeKind::Valley,
            })
            .unwrap();
        run_once(&store, &dir).unwrap();
        let expected = store.lock().unwrap().replay_inputs().0;

        // 異常終了を模して、まっさらなstoreへ復元する
        let fresh = Mutex::new(DocumentStore::default());
        let info = check(&dir).expect("自動保存が残っている");
        assert_eq!(
            info.document_path,
            Some(doc_path.to_string_lossy().into_owned())
        );
        let view = restore(&fresh, &dir).unwrap().expect("復元できる");
        assert_eq!(view.doc, expected, "作業中だった内容が戻る");
        let s = fresh.lock().unwrap();
        assert_eq!(s.current_path(), Some(doc_path), "元の保存先を引き継ぐ");
        assert!(s.is_dirty(), "まだ保存していない内容なので未保存扱い");
        drop(s);
        assert!(check(&dir).is_none(), "復元後は同じ提案を繰り返さない");
    }

    /// 復元した候補は、明示保存に成功するまで残す。復元は候補の削除ではない。
    #[test]
    fn restored_candidate_is_deleted_only_after_a_successful_save() {
        let dir = temp_dir("restore_then_save");
        let document_path = dir.join("鶴.ori3");
        let previous = named_store_with_pending_edit(&dir, "鶴.ori3");
        run_once(&previous, &dir).unwrap();
        prepare_session(&dir).unwrap();
        let candidate = PathBuf::from(check(&dir).expect("持ち越し候補がある").autosave_path);

        let restored = Mutex::new(DocumentStore::default());
        restore(&restored, &dir).unwrap().expect("復元できる");
        assert!(candidate.is_file(), "復元だけで候補を消してはいけない");
        assert!(
            check_all(&dir).unwrap().choices.is_empty(),
            "復元中の同じ候補は重ねて出さない"
        );

        restored.lock().unwrap().save(Some(&document_path)).unwrap();
        discard_after_save(&dir, Some(&document_path));
        assert!(!candidate.exists(), "保存成功後にだけ候補を消す");
    }

    /// 「あとで確認する」の間に編集した今の作業を、過去の候補を復元する操作で失わない。
    #[test]
    fn restoring_a_carried_candidate_keeps_the_current_active_work() {
        let dir = temp_dir("restore_after_later");
        let previous = named_store_with_pending_edit(&dir, "前回の鶴.ori3");
        run_once(&previous, &dir).unwrap();
        prepare_session(&dir).unwrap();

        let current = named_store_with_pending_edit(&dir, "いまの水風船.ori3");
        run_once(&current, &dir).unwrap();
        let id = check_all(&dir).unwrap().choices[0]
            .candidate_id
            .expect("前回の候補には番号がある");

        let restored = Mutex::new(DocumentStore::default());
        restore_candidate(&restored, &dir, Some(id))
            .unwrap()
            .expect("前回の候補を復元できる");

        let choices = check_all(&dir).unwrap().choices;
        assert_eq!(choices.len(), 1, "今の作業を別の候補として残す");
        assert_eq!(
            choices[0].document_path,
            Some(dir.join("いまの水風船.ori3").to_string_lossy().into_owned())
        );
    }

    /// 複数候補のうち1件を復元しても、選ばなかった候補は次の再照会で表示し続ける。
    #[test]
    fn restoring_one_of_multiple_candidates_keeps_the_other_choice_visible() {
        let dir = temp_dir("restore_one_keep_other_visible");
        for name in ["折り鶴.ori3", "水風船.ori3"] {
            let store = named_store_with_pending_edit(&dir, name);
            run_once(&store, &dir).unwrap();
            prepare_session(&dir).unwrap();
        }

        let before = check_all(&dir).unwrap();
        assert_eq!(before.choices.len(), 2, "復元前は2件とも選べる");
        let crane_path = dir.join("折り鶴.ori3").to_string_lossy().into_owned();
        let balloon_path = dir.join("水風船.ori3").to_string_lossy().into_owned();
        let crane = before
            .choices
            .iter()
            .find(|choice| choice.document_path.as_deref() == Some(crane_path.as_str()))
            .expect("折り鶴の候補がある");
        let crane_id = crane.candidate_id.expect("持ち越し候補には番号がある");
        let crane_payload = PathBuf::from(&crane.autosave_path);
        let balloon_id = before
            .choices
            .iter()
            .find(|choice| choice.document_path.as_deref() == Some(balloon_path.as_str()))
            .and_then(|choice| choice.candidate_id)
            .expect("水風船の候補に番号がある");

        let restored = Mutex::new(DocumentStore::default());
        restore_candidate(&restored, &dir, Some(crane_id))
            .unwrap()
            .expect("折り鶴を復元できる");

        let after = check_all(&dir).unwrap();
        assert_eq!(after.choices.len(), 1, "選ばなかった1件を表示し続ける");
        assert_eq!(after.overflow_count, 0);
        assert_eq!(after.choices[0].candidate_id, Some(balloon_id));
        assert_eq!(
            after.choices[0].document_path.as_deref(),
            Some(balloon_path.as_str()),
            "水風船が次の復旧画面に残る"
        );
        assert!(
            crane_payload.is_file(),
            "復元した折り鶴も保存成功までは控えを消さない"
        );
    }

    /// 通常の作品を保存しても持ち越し候補は消さず、復元した作品の保存だけは復元元を消す。
    #[test]
    fn successful_save_discards_only_the_restored_source_candidate() {
        let dir = temp_dir("save_discards_only_restored_source");
        for name in ["折り鶴.ori3", "水風船.ori3"] {
            let store = named_store_with_pending_edit(&dir, name);
            run_once(&store, &dir).unwrap();
            prepare_session(&dir).unwrap();
        }

        let before = check_all(&dir).unwrap().choices;
        assert_eq!(before.len(), 2, "保存前は2件とも持ち越している");
        let crane_path = dir.join("折り鶴.ori3").to_string_lossy().into_owned();
        let balloon_path = dir.join("水風船.ori3").to_string_lossy().into_owned();
        let crane = before
            .iter()
            .find(|choice| choice.document_path.as_deref() == Some(crane_path.as_str()))
            .expect("折り鶴の候補がある");
        let crane_id = crane.candidate_id.expect("折り鶴の候補番号がある");
        let crane_payload = PathBuf::from(&crane.autosave_path);
        let balloon = before
            .iter()
            .find(|choice| choice.document_path.as_deref() == Some(balloon_path.as_str()))
            .expect("水風船の候補がある");
        let balloon_id = balloon.candidate_id.expect("水風船の候補番号がある");
        let balloon_payload = PathBuf::from(&balloon.autosave_path);

        // 復旧候補とは無関係な今回の作品を明示保存しても、持ち越し候補は選んでいない。
        let current = store_with_edit();
        current
            .lock()
            .unwrap()
            .save(Some(&dir.join("今回の作品.ori3")))
            .unwrap();
        let current_path = current.lock().unwrap().current_path();
        discard_after_save(&dir, current_path.as_deref());
        let after_unrelated_save = check_all(&dir).unwrap().choices;
        assert_eq!(
            after_unrelated_save.len(),
            2,
            "復元していない作品の保存で持ち越し候補を消してはいけない"
        );

        let restored = Mutex::new(DocumentStore::default());
        restore_candidate(&restored, &dir, Some(crane_id))
            .unwrap()
            .expect("折り鶴を復元できる");
        restored
            .lock()
            .unwrap()
            .save(Some(&dir.join("折り鶴.ori3")))
            .unwrap();
        let restored_path = restored.lock().unwrap().current_path();
        discard_after_save(&dir, restored_path.as_deref());

        let after_restored_save = check_all(&dir).unwrap().choices;
        assert_eq!(after_restored_save.len(), 1, "未選択の候補は1件残す");
        assert_eq!(after_restored_save[0].candidate_id, Some(balloon_id));
        assert!(!crane_payload.exists(), "保存した復元元だけを消す");
        assert!(balloon_payload.is_file(), "水風船の候補は消さない");
    }

    /// 破棄すると自動保存ファイルも目印も消える。
    #[test]
    fn discard_removes_autosave_file() {
        let dir = temp_dir("discard");
        let store = store_with_edit();
        run_once(&store, &dir).unwrap();
        let autosave = PathBuf::from(check(&dir).unwrap().autosave_path);
        assert!(autosave.is_file());

        discard_after_save(&dir, None);
        assert!(!autosave.exists(), "自動保存ファイルが残っている");
        assert!(!marker_path(&dir).exists(), "目印が残っている");
        assert!(check(&dir).is_none());
        assert!(restore(&store, &dir).unwrap().is_none());
    }

    #[test]
    fn clean_exit_discards_but_dirty_exit_keeps_the_autosave() {
        let dir = temp_dir("exit");
        let store = store_with_edit();
        run_once(&store, &dir).unwrap();
        let autosave = PathBuf::from(check(&dir).unwrap().autosave_path);

        // 未保存の通常終了はクラッシュ時と同じく、次回の復旧に残す。
        discard_if_clean(&store, &dir);
        assert!(autosave.is_file(), "未保存なのに自動保存を消している");

        // 保存済みなら正常終了なので片付ける。
        store
            .lock()
            .unwrap()
            .save(Some(&dir.join("保存済み.ori3")))
            .unwrap();
        discard_if_clean(&store, &dir);
        assert!(!autosave.exists(), "保存済みなのに自動保存が残っている");
    }

    #[test]
    fn tampered_marker_never_deletes_an_unrelated_file() {
        let dir = temp_dir("tampered_marker");
        let unrelated =
            std::env::temp_dir().join(format!("ori3_unrelated_{}.autosave", std::process::id()));
        let unrelated_document = document_path_of(&unrelated).unwrap();
        std::fs::write(&unrelated_document, "別の作品").unwrap();
        std::fs::write(&unrelated, "消してはいけない").unwrap();
        std::fs::write(marker_path(&dir), unrelated.to_string_lossy().as_bytes()).unwrap();

        discard_after_save(&dir, None);

        assert!(
            unrelated.is_file(),
            "目印の改変で関係ないファイルを消している"
        );
        assert!(!marker_path(&dir).exists(), "危険な目印は片付ける");
        std::fs::remove_file(unrelated).ok();
        std::fs::remove_file(unrelated_document).ok();
    }

    /// 書き出し先の決め方(保存先があれば隣、無題ならアプリデータ配下)。
    #[test]
    fn target_path_uses_suffix_or_app_data() {
        let dir = Path::new("/データ");
        let named = target_path(Some(Path::new("/作品/鶴.ori3")), dir);
        assert!(named.to_string_lossy().ends_with("鶴.ori3.autosave"));
        assert_eq!(
            document_path_of(&named),
            Some(PathBuf::from("/作品/鶴.ori3"))
        );
        let untitled = target_path(None, dir);
        assert_eq!(untitled, dir.join(UNTITLED_FILE));
        assert_eq!(document_path_of(&untitled), None);
    }

    /// 旧形式の目印が残っている利用者も、候補を消さずに複数候補の索引へ移せる。
    #[test]
    fn legacy_single_candidate_is_migrated_without_deleting_its_source() {
        let dir = temp_dir("legacy_migration");
        let store = store_with_edit();
        let (_, doc) = store
            .lock()
            .unwrap()
            .autosave_snapshot()
            .expect("未保存の内容を用意した");
        let legacy = target_path(None, &dir);
        let text = serde_json::to_string_pretty(&doc).unwrap();
        std::fs::write(&legacy, &text).unwrap();
        std::fs::write(marker_path(&dir), legacy.to_string_lossy().as_bytes()).unwrap();

        prepare_session(&dir).unwrap();

        let choices = check_all(&dir).unwrap().choices;
        assert_eq!(choices.len(), 1, "以前の1件を候補として引き継ぐ");
        assert_eq!(choices[0].document_path, None);
        assert!(Path::new(&choices[0].autosave_path).is_file());
        assert!(legacy.is_file(), "移行に失敗しても旧payloadを失わない");
        assert!(
            marker_path(&dir).is_file(),
            "移行に失敗しても旧目印を失わない"
        );
    }

    /// 復旧画面で何も選ばないまま正常終了しても、移行済みの旧形式候補を消さない。
    #[test]
    fn clean_exit_keeps_migrated_legacy_candidate_without_user_choice() {
        let dir = temp_dir("clean_exit_keeps_legacy_candidate");
        let previous = store_with_edit();
        let (_, doc) = previous
            .lock()
            .unwrap()
            .autosave_snapshot()
            .expect("旧形式へ置く未保存の内容を用意した");
        let legacy = target_path(None, &dir);
        let text = serde_json::to_string_pretty(&doc).unwrap();
        std::fs::write(&legacy, &text).unwrap();
        std::fs::write(marker_path(&dir), legacy.to_string_lossy().as_bytes()).unwrap();

        prepare_session(&dir).unwrap();
        let before = check_all(&dir).unwrap().choices;
        assert_eq!(before.len(), 1, "旧形式を持ち越し候補へ移行できる");
        let candidate_payload = PathBuf::from(&before[0].autosave_path);

        let clean_store = Mutex::new(DocumentStore::default());
        discard_if_clean(&clean_store, &dir);

        let after = check_all(&dir).unwrap().choices;
        assert_eq!(
            after.len(),
            1,
            "何も選ばない正常終了で持ち越し候補を消してはいけない"
        );
        assert!(candidate_payload.is_file(), "移行済みpayloadを残す");
        assert!(legacy.is_file(), "互換用の旧payloadも残す");
        assert!(marker_path(&dir).is_file(), "互換用の旧目印も残す");
    }

    #[test]
    fn test_app_data_dir_override_is_opt_in_and_preserves_the_unset_path() {
        assert_eq!(test_app_data_dir_override(None), None);
        assert_eq!(test_app_data_dir_override(Some(OsString::new())), None);
        assert_eq!(
            test_app_data_dir_override(Some(OsString::from("C:\\隔離\\autosave"))),
            Some(PathBuf::from("C:\\隔離\\autosave"))
        );
    }

    /// 持ち越し候補だけを読み、現行作業枠は数えない。
    fn recovery_infos(app_data: &Path) -> Vec<RecoveryInfo> {
        check_all(app_data).unwrap().choices
    }

    fn named_store_with_pending_edit(app_data: &Path, name: &str) -> Mutex<DocumentStore> {
        let store = store_with_edit();
        let path = app_data.join(name);
        store.lock().unwrap().save(Some(&path)).unwrap();
        store
            .lock()
            .unwrap()
            .apply_edit(EditOp::AddSegment {
                a: [1.0, 0.0],
                b: [0.0, 1.0],
                kind: EdgeKind::Valley,
            })
            .unwrap();
        store
    }

    /// 段階3(1, 2): 前回の無題作品を提示したまま、別の無題作品を編集して
    /// 30秒後の自動保存が走っても、両方を選べる必要がある。
    #[test]
    fn recovery_choices_keep_untitled_work_before_new_untitled_autosave() {
        let dir = temp_dir("red_untitled_previous_and_current");
        let previous = store_with_edit();
        run_once(&previous, &dir).unwrap(); // 前回の異常終了で残った内容
        prepare_session(&dir).unwrap();

        let current = store_with_edit();
        assert!(
            run_once(&current, &dir).unwrap(),
            "別の無題作品も30秒後に控える"
        );
        // 次回起動時にも、前回と今回の両方を候補として提示できる。
        prepare_session(&dir).unwrap();

        let choices = recovery_infos(&dir);
        assert_eq!(
            choices.len(),
            2,
            "前回の無題作品と今の無題作品を、どちらも復旧画面で選べる"
        );
        assert_eq!(
            choices
                .iter()
                .filter(|choice| choice.document_path.is_none())
                .count(),
            2,
            "無題の2件を1件へ上書きしてはいけない"
        );
    }

    /// 段階3(1, 3): 別名の作品では本体が残っても、前回の内容が復旧画面から
    /// 選べなくなってはいけない。
    #[test]
    fn recovery_choices_keep_named_work_before_different_named_autosave() {
        let dir = temp_dir("red_named_previous_and_current");
        let previous_path = dir.join("前回の鶴.ori3");
        let current_path = dir.join("いまのやっこさん.ori3");
        let previous = named_store_with_pending_edit(&dir, "前回の鶴.ori3");
        run_once(&previous, &dir).unwrap();
        prepare_session(&dir).unwrap();

        let current = named_store_with_pending_edit(&dir, "いまのやっこさん.ori3");
        assert!(run_once(&current, &dir).unwrap());
        prepare_session(&dir).unwrap();

        let choices = recovery_infos(&dir);
        let paths: Vec<_> = choices
            .iter()
            .filter_map(|choice| choice.document_path.as_deref())
            .collect();
        assert_eq!(choices.len(), 2, "以前と現在の2件を選べる");
        assert!(
            paths.contains(&previous_path.to_string_lossy().as_ref()),
            "前回の鶴が復旧画面から消えてはいけない"
        );
        assert!(
            paths.contains(&current_path.to_string_lossy().as_ref()),
            "今のやっこさんも控えられる"
        );
    }

    /// 段階3(4, 6): 未解決の3件があっても、今の作業を控え続け、以前の3件を
    /// 自動削除してはいけない。
    #[test]
    fn three_unresolved_choices_do_not_stop_the_current_autosave() {
        let dir = temp_dir("red_three_unresolved_and_current");
        let old_paths: Vec<_> = ["鶴.ori3", "やっこさん.ori3", "鳥の基本形.ori3"]
            .into_iter()
            .map(|name| {
                let store = named_store_with_pending_edit(&dir, name);
                run_once(&store, &dir).unwrap();
                prepare_session(&dir).unwrap();
                dir.join(name).to_string_lossy().into_owned()
            })
            .collect();
        let current = store_with_edit();
        assert!(
            run_once(&current, &dir).unwrap(),
            "未解決3件でも今の作業の30秒ごとの控えを止めない"
        );
        assert!(active_path(&dir).is_file(), "今の作業の専用枠は常に書ける");

        let choices = recovery_infos(&dir);
        assert_eq!(choices.len(), 3, "未解決の3件をすべて残す");
        for path in old_paths {
            assert!(
                choices
                    .iter()
                    .any(|choice| choice.document_path.as_deref() == Some(path.as_str())),
                "未解決の候補を今の作業より先に消してはいけない: {path}"
            );
        }
    }

    /// 段階3(5): 4件目以降も黙って消さず、すべて利用者が選べる状態で残す。
    #[test]
    fn four_or_more_unresolved_choices_are_never_deleted_silently() {
        let dir = temp_dir("red_four_unresolved");
        let names = [
            "鶴.ori3",
            "やっこさん.ori3",
            "鳥の基本形.ori3",
            "水風船.ori3",
        ];
        for name in names {
            let store = named_store_with_pending_edit(&dir, name);
            run_once(&store, &dir).unwrap();
            prepare_session(&dir).unwrap();
        }

        let choices = recovery_infos(&dir);
        assert_eq!(
            choices.len(),
            4,
            "4件以上になっても、利用者が破棄するまで候補を消さない"
        );
        for name in names {
            let path = dir.join(name).to_string_lossy().into_owned();
            assert!(
                choices
                    .iter()
                    .any(|choice| choice.document_path.as_deref() == Some(path.as_str())),
                "候補を黙って消してはいけない: {name}"
            );
        }
    }
}
