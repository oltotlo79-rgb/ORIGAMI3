//! 自動保存とクラッシュ復旧(SYS-003)。
//!
//! 30秒ごとに、未保存の変更があるときだけ作業中の内容をアプリデータ配下の
//! 専用ファイルへ書き出す。実行中の作業は常に1件の専用枠へ書き、異常終了した
//! 作業は次回起動時に持ち越し候補へ移す。持ち越し候補は利用者が破棄するか、
//! 復元後に明示保存するまで消さない。
//!
//! 設計規約: 定期保存は複製を取る一瞬だけstoreをロックし、JSON化と書き出しは外で行う。
//! 復元だけは最終編集の控えと画面入替えを同じcommitにするため、入替え完了まで保持する。
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
/// indexのatomic確定前に止まっても、payload単体から表示情報を戻すための予約field。
/// `SavedDocument`は未知fieldを無視するため、候補payloadはそのまま作品として読める。
const PAYLOAD_METADATA_FIELD: &str = "_ori3_autosave";

/// background保存と画面commandが同じ索引をread-modify-writeする順序を直列化する。
/// DocumentStoreのlockとは分け、JSON化・disk I/O中に作品本体を止めない。
static AUTOSAVE_FILE_IO: Mutex<()> = Mutex::new(());

fn lock_autosave_files() -> std::sync::MutexGuard<'static, ()> {
    AUTOSAVE_FILE_IO
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct EmbeddedAutosaveMetadata {
    document_path: Option<String>,
    saved_at_ms: u64,
    step_count: usize,
    /// index確定前に止まっても、復元中だった元候補との結び付きをpayloadから戻す。
    /// 旧payloadには無いため、欠けている場合は未復元の作業として扱う。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_candidate_id: Option<u64>,
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

/// 壊れた索引を上書きする前に、利用者の調査・手動救出用として原文を残す。
/// 同じprocess・同じミリ秒でも既存backupを上書きしない。
fn backup_corrupt_index(app_data: &Path, bytes: &[u8]) -> Result<(), String> {
    let stamp = now_ms().unwrap_or(0);
    for sequence in 0_u32.. {
        let path = app_data.join(format!(
            "autosave-index.{stamp}.{}.{}.corrupt",
            std::process::id(),
            sequence
        ));
        if path.exists() {
            continue;
        }
        write_atomic(&path, bytes)
            .map_err(|e| format!("壊れた自動保存の索引を退避できませんでした: {e}"))?;
        return Ok(());
    }
    unreachable!("u32の全候補名が使用済みになることはない")
}

fn payload_saved_at_ms(path: &Path) -> Result<u64, String> {
    if let Some(metadata) = payload_metadata(path) {
        return Ok(metadata.saved_at_ms);
    }
    Ok(std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(now_ms()?))
}

fn payload_metadata(path: &Path) -> Option<EmbeddedAutosaveMetadata> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    serde_json::from_value(value.get(PAYLOAD_METADATA_FIELD)?.clone()).ok()
}

/// JSONが途中で切れていても候補自体は隠さない。復元時に個別errorとして扱い、
/// 利用者が他候補を復元したり、この候補だけを破棄したりできるようにする。
fn payload_step_count(path: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_document(&text)
        .ok()
        .map(|saved| saved.document.sequence.len())
}

fn candidate_ids_on_disk(app_data: &Path) -> Result<Vec<u64>, String> {
    let mut ids = Vec::new();
    let entries = match std::fs::read_dir(app_data.join(CANDIDATES_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
        Err(error) => {
            return Err(format!(
                "自動保存の候補ディレクトリを読み込めませんでした: {error}"
            ));
        }
    };
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("自動保存の候補ディレクトリを読み込めませんでした: {e}"))?;
        if !entry
            .file_type()
            .map_err(|e| format!("自動保存の候補を確認できませんでした: {e}"))?
            .is_file()
        {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(id) = name
            .strip_suffix(".ori3")
            .and_then(|stem| stem.parse::<u64>().ok())
        else {
            continue;
        };
        if name != format!("{id}.ori3") {
            continue;
        }
        ids.push(id);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// 索引のatomic rename前に強制終了した場合でも、完成済みpayloadから候補を戻す。
/// parse不能payloadもstep_count=0の個別候補として保持し、黙って削除しない。
fn reconcile_payload_files(app_data: &Path, index: &mut AutosaveIndex) -> Result<bool, String> {
    let mut changed = false;
    for id in candidate_ids_on_disk(app_data)? {
        let path = candidate_path(app_data, id);
        let payload_metadata = payload_metadata(&path);
        let saved_at_ms = payload_saved_at_ms(&path)?;
        let step_count = payload_step_count(&path);
        if let Some(candidate) = index
            .carried
            .iter_mut()
            .find(|candidate| candidate.id == id)
        {
            if let Some(step_count) = step_count
                && candidate.step_count != step_count
            {
                candidate.step_count = step_count;
                changed = true;
            }
            if let Some(payload_metadata) = payload_metadata.as_ref() {
                if candidate.saved_at_ms != payload_metadata.saved_at_ms {
                    candidate.saved_at_ms = payload_metadata.saved_at_ms;
                    changed = true;
                }
                if candidate.document_path != payload_metadata.document_path {
                    candidate.document_path = payload_metadata.document_path.clone();
                    changed = true;
                }
            }
        } else {
            index.carried.push(StoredCandidate {
                id,
                document_path: payload_metadata.and_then(|metadata| metadata.document_path),
                saved_at_ms,
                step_count: step_count.unwrap_or(0),
            });
            changed = true;
        }
        let next = id.saturating_add(1);
        if index.next_candidate_id < next {
            index.next_candidate_id = next;
            changed = true;
        }
    }

    let active_path = active_path(app_data);
    if !active_path.is_file() {
        if index.active.take().is_some() {
            changed = true;
        }
        return Ok(changed);
    }

    if index.active.is_none() {
        let metadata = payload_metadata(&active_path);
        // stale activeと既存候補が同じbytesでも自動統合しない。異なる無題作品が
        // 偶然同じ内容だった可能性を捨てず、余分な候補になり得る安全側を選ぶ。
        index.active = Some(ActiveSnapshot {
            document_path: metadata
                .as_ref()
                .and_then(|metadata| metadata.document_path.clone()),
            saved_at_ms: payload_saved_at_ms(&active_path)?,
            step_count: payload_step_count(&active_path).unwrap_or(0),
            visible_for_legacy_check: true,
            source_candidate_id: metadata.and_then(|metadata| metadata.source_candidate_id),
        });
        return Ok(true);
    }

    let step_count = payload_step_count(&active_path);
    let payload_metadata = payload_metadata(&active_path);
    let active = index.active.as_mut().expect("直前にSomeを確認した");
    if let Some(step_count) = step_count
        && active.step_count != step_count
    {
        active.step_count = step_count;
        changed = true;
    }
    if let Some(payload_metadata) = payload_metadata {
        if active.saved_at_ms != payload_metadata.saved_at_ms {
            active.saved_at_ms = payload_metadata.saved_at_ms;
            changed = true;
        }
        if active.document_path != payload_metadata.document_path {
            active.document_path = payload_metadata.document_path;
            changed = true;
        }
        if payload_metadata.source_candidate_id.is_some()
            && active.source_candidate_id != payload_metadata.source_candidate_id
        {
            active.source_candidate_id = payload_metadata.source_candidate_id;
            changed = true;
        }
    }
    Ok(changed)
}

/// 起動時だけ、索引の破損・欠落をpayloadから安全側へ復元する。
/// 壊れた索引は必ず退避してから再構築し、正常な候補を1件も空indexで隠さない。
fn startup_index(app_data: &Path) -> Result<Option<AutosaveIndex>, String> {
    let path = index_path(app_data);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("自動保存の索引を読み込めませんでした: {error}")),
    };
    let had_index = bytes.is_some();
    let mut valid_index = false;
    let mut index = if let Some(bytes) = bytes {
        match serde_json::from_slice::<AutosaveIndex>(&bytes) {
            Ok(index) if index.version == INDEX_VERSION => {
                valid_index = true;
                index
            }
            _ => {
                backup_corrupt_index(app_data, &bytes)?;
                AutosaveIndex::default()
            }
        }
    } else {
        AutosaveIndex::default()
    };

    // 先に現行の候補IDを拾い、旧形式を空いているIDへ移す。順序を逆にすると、
    // 壊れた索引の復旧時に旧形式が既存の`1.ori3`を上書きし得る。
    let mut found_payload = reconcile_payload_files(app_data, &mut index)?;
    if !valid_index {
        found_payload |= import_legacy_candidate(app_data, &mut index)?;
    }
    if !had_index && !found_payload && index.active.is_none() && index.carried.is_empty() {
        return Ok(None);
    }
    Ok(Some(index))
}

fn write_index(app_data: &Path, index: &AutosaveIndex) -> Result<(), String> {
    let text = serde_json::to_string_pretty(index)
        .map_err(|e| format!("自動保存の索引を作成できませんでした: {e}"))?;
    write_atomic(index_path(app_data).as_path(), text.as_bytes())
        .map_err(|e| format!("自動保存の索引を書き込めませんでした: {e}"))
}

fn fresh_candidate_id(index: &mut AutosaveIndex, app_data: &Path) -> u64 {
    let mut id = index.next_candidate_id.max(1);
    loop {
        if !index.carried.iter().any(|candidate| candidate.id == id)
            && !candidate_path(app_data, id).exists()
        {
            index.next_candidate_id = id.checked_add(1).unwrap_or(1);
            return id;
        }
        id = id.checked_add(1).unwrap_or(1);
    }
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
    let payload = std::fs::read(&legacy_path)
        .map_err(|e| format!("以前の自動保存を読み込めませんでした: {e}"))?;
    let id = fresh_candidate_id(index, app_data);
    std::fs::create_dir_all(app_data.join(CANDIDATES_DIR))
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(candidate_path(app_data, id).as_path(), &payload)
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
        step_count: std::str::from_utf8(&payload)
            .ok()
            .and_then(|text| parse_document(text).ok())
            .map_or(0, |saved| saved.document.sequence.len()),
    });
    Ok(true)
}

fn parsed_payload(payload: &[u8]) -> Option<SavedDocument> {
    std::str::from_utf8(payload)
        .ok()
        .and_then(|text| parse_document(text).ok())
}

/// 復元中の元候補へactiveを戻す。activeが壊れていて元候補が正常なら上書きせず、
/// callerへfalseを返して壊れたactiveを別候補として保持させる。
fn update_source_candidate(
    app_data: &Path,
    candidate: &mut StoredCandidate,
    active: &ActiveSnapshot,
    payload: &[u8],
) -> Result<bool, String> {
    let path = candidate_path(app_data, candidate.id);
    let active_document = parsed_payload(payload);
    let existing_payload = std::fs::read(&path).ok();
    let existing_document = existing_payload.as_deref().and_then(parsed_payload);
    if existing_payload.is_some() && (active_document.is_none() || existing_document.is_none()) {
        // 読めない既存bytesは将来schemaかもしれない。読めないactiveと同様に
        // 黙って上書きせず、両方を個別候補として残す。
        return Ok(false);
    }
    if active_document.is_some() && active_document == existing_document {
        // 復元しただけで作品内容が変わっていない。元候補のbytesと表示日時を保つ。
        return Ok(true);
    }
    write_atomic(&path, payload).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    candidate.document_path = active.document_path.clone();
    candidate.saved_at_ms = active.saved_at_ms;
    candidate.step_count = active_document.map_or(0, |saved| saved.document.sequence.len());
    Ok(true)
}

/// 起動ごとに、前回動いていた作業枠を利用者が選べる持ち越し候補へ移す。
/// 持ち越し件数が3件を超えても削除しない。今の作業枠は常に空けておく。
fn prepare_session(app_data: &Path) -> Result<(), String> {
    let _file_io = lock_autosave_files();
    std::fs::create_dir_all(app_data).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    let Some(mut index) = startup_index(app_data)? else {
        return Ok(());
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
        && update_source_candidate(app_data, candidate, &active, &payload)?
    {
        write_index(app_data, &index)?;
        std::fs::remove_file(active_path(app_data)).ok();
        return Ok(());
    }
    let id = fresh_candidate_id(&mut index, app_data);
    std::fs::create_dir_all(app_data.join(CANDIDATES_DIR))
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(candidate_path(app_data, id).as_path(), &payload)
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    index.carried.push(StoredCandidate {
        id,
        document_path: active.document_path,
        saved_at_ms: active.saved_at_ms,
        step_count: parsed_payload(&payload).map_or(0, |saved| saved.document.sequence.len()),
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
        && update_source_candidate(app_data, candidate, &active, &payload)?
    {
        return Ok(());
    }
    let id = fresh_candidate_id(index, app_data);
    std::fs::create_dir_all(app_data.join(CANDIDATES_DIR))
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(candidate_path(app_data, id).as_path(), &payload)
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    index.carried.push(StoredCandidate {
        id,
        document_path: active.document_path.clone(),
        saved_at_ms: active.saved_at_ms,
        step_count: parsed_payload(&payload).map_or(0, |saved| saved.document.sequence.len()),
    });
    Ok(())
}

/// 未保存の現在作品を別作品へ入れ替える前に、今回の作業枠を持ち越し候補へ移す。
///
/// 復元した候補は`active.source_candidate_id`で元候補と結ばれている。そのまま
/// 新規作成・別作品の読込みへ進むと、次の自動保存や明示保存が別作品を同じ候補と
/// 誤認して上書き・削除するため、作品を入れ替える前に結び付きを外す。
/// 索引を確定してからactive payloadを消すので、途中で失敗しても内容を失わない。
pub fn preserve_before_document_change(
    store: &Mutex<DocumentStore>,
    app_data: &Path,
) -> Result<(), String> {
    let _file_io = lock_autosave_files();
    // 最後の30秒以降の編集も、作品を入れ替える直前に同じrun_once経路で確定する。
    if !run_once_unlocked(store, app_data)? {
        return Ok(());
    }
    let Some(mut index) = read_index(app_data)? else {
        return Ok(());
    };
    if index.active.is_none() {
        return Ok(());
    }
    preserve_active_as_candidate(app_data, &mut index)?;
    index.active = None;
    write_index(app_data, &index)?;
    std::fs::remove_file(active_path(app_data)).ok();
    Ok(())
}

/// 自動保存を1回行う。未保存の変更が無ければ何もせずfalseを返す。
/// ロックは複製を取る間だけ持ち、JSON化と書き出しはロックの外で行う。
pub fn run_once(store: &Mutex<DocumentStore>, app_data: &Path) -> Result<bool, String> {
    let _file_io = lock_autosave_files();
    run_once_unlocked(store, app_data)
}

fn run_once_unlocked(store: &Mutex<DocumentStore>, app_data: &Path) -> Result<bool, String> {
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

/// 30秒の待機と1回の自動保存を同じ製品経路として実行する。
/// `wait`を注入できるため、検査では30秒を実際に待たず境界を確認できる。
fn run_after_interval(
    store: &Mutex<DocumentStore>,
    app_data: &Path,
    wait: impl FnOnce(Duration),
) -> Result<bool, String> {
    wait(INTERVAL);
    run_once(store, app_data)
}

fn snapshot_json(
    doc: &SavedDocument,
    metadata: &EmbeddedAutosaveMetadata,
) -> Result<String, String> {
    let mut value = serde_json::to_value(doc)
        .map_err(|e| format!("自動保存データの作成に失敗しました: {e}"))?;
    value
        .as_object_mut()
        .expect("SavedDocumentはJSON objectとして直列化される")
        .insert(
            PAYLOAD_METADATA_FIELD.to_owned(),
            serde_json::to_value(metadata)
                .map_err(|e| format!("自動保存データの作成に失敗しました: {e}"))?,
        );
    serde_json::to_string_pretty(&value)
        .map_err(|e| format!("自動保存データの作成に失敗しました: {e}"))
}

/// 複製した作品を今回起動中だけの専用枠へ書く。持ち越し候補は変更しないため、
/// 利用者が復旧を決める前に別作品を編集しても以前の候補は上書きされない。
fn write_snapshot(
    doc: &SavedDocument,
    doc_path: Option<&Path>,
    app_data: &Path,
) -> Result<(), String> {
    let document_path = doc_path.map(|path| path.to_string_lossy().into_owned());
    // 復元直後など、dirtyだが内容が直前のactiveと同一なら元payloadの日時を保つ。
    // 作品切替前の安全な再確認だけで、元候補のbytesを無意味に書き換えないため。
    let previous_metadata = std::fs::read_to_string(active_path(app_data))
        .ok()
        .and_then(|text| {
            let saved = parse_document(&text).ok()?;
            (saved == *doc).then(|| payload_metadata(&active_path(app_data)))?
        })
        .filter(|metadata| metadata.document_path == document_path);
    let saved_at_ms = previous_metadata
        .map(|metadata| metadata.saved_at_ms)
        .map_or_else(now_ms, Ok)?;
    let mut index = read_index(app_data)?.unwrap_or_default();
    let source_candidate_id = index
        .active
        .as_ref()
        .and_then(|active| active.source_candidate_id);
    let metadata = EmbeddedAutosaveMetadata {
        document_path,
        saved_at_ms,
        step_count: doc.document.sequence.len(),
        source_candidate_id,
    };
    let json = snapshot_json(doc, &metadata)?;
    std::fs::create_dir_all(app_data).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(active_path(app_data).as_path(), json.as_bytes())
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    index.active = Some(ActiveSnapshot {
        document_path: metadata.document_path,
        saved_at_ms,
        step_count: metadata.step_count,
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
    let _file_io = lock_autosave_files();
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

#[cfg(test)]
static TEST_PAUSE_AFTER_RESTORE_ACTIVE_WRITE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
fn arm_pause_after_restore_active_write() {
    TEST_PAUSE_AFTER_RESTORE_ACTIVE_WRITE.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
fn pause_after_restore_active_write_if_armed() -> Result<(), String> {
    if !TEST_PAUSE_AFTER_RESTORE_ACTIVE_WRITE.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    let ready = std::env::var_os("ORI3_TEST_AUTOSAVE_PROCESS_READY")
        .map(PathBuf::from)
        .ok_or_else(|| "強制終了検査のready pathがありません".to_owned())?;
    std::fs::write(ready, b"ready")
        .map_err(|e| format!("強制終了検査のreadyを書けませんでした: {e}"))?;
    loop {
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

/// 利用者が選んだ候補を現在の作業枠へ写して復元する。
/// 元の候補は明示保存まで残すため、復元直後に再び異常終了しても内容を失わない。
pub fn restore_candidate(
    store: &Mutex<DocumentStore>,
    app_data: &Path,
    candidate_id: Option<u64>,
) -> Result<Option<DocumentView>, String> {
    let _file_io = lock_autosave_files();
    let Some(index) = read_index(app_data)? else {
        return Ok(None);
    };
    let active_source = index
        .active
        .as_ref()
        .and_then(|active| active.source_candidate_id);
    let id = match candidate_id {
        Some(id) if Some(id) == active_source => return Ok(None),
        Some(id) => Some(id),
        None => index
            .carried
            .iter()
            .find(|candidate| Some(candidate.id) != active_source)
            .map(|candidate| candidate.id),
    };
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
    let saved_at_ms = now_ms()?;
    let step_count = doc.document.sequence.len();
    let metadata = EmbeddedAutosaveMetadata {
        document_path: candidate.document_path.clone(),
        saved_at_ms,
        step_count,
        source_candidate_id: Some(id),
    };
    let active_json = snapshot_json(&doc, &metadata)?;

    // 現在作品の最後の編集から画面の入替えまで同じstore lockを保持する。
    // 30秒未満の編集もここで控え、別commandの編集が間へ入って消える余地を作らない。
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((document_path, current)) = store.autosave_snapshot() {
        write_snapshot(&current, document_path.as_deref(), app_data)?;
    }
    let Some(mut index) = read_index(app_data)? else {
        return Ok(None);
    };
    if !index.carried.iter().any(|candidate| candidate.id == id) {
        return Ok(None);
    }

    // 現行作業を候補へ確定し、activeとの結び付きをいったんatomicに外す。
    // このindex確定後なら、次のpayload確定中に止まっても旧sourceへ上書きしない。
    preserve_active_as_candidate(app_data, &mut index)?;
    index.active = None;
    write_index(app_data, &index)?;
    std::fs::remove_file(active_path(app_data)).ok();

    // 選択候補のpayloadにはsource IDも埋める。payload確定後・index確定前に
    // 強制終了しても、次回起動が選択候補との結び付きを正しく再構築できる。
    write_atomic(active_path(app_data).as_path(), active_json.as_bytes())
        .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    #[cfg(test)]
    pause_after_restore_active_write_if_armed()?;
    index.active = Some(ActiveSnapshot {
        document_path: metadata.document_path,
        saved_at_ms,
        step_count,
        visible_for_legacy_check: false,
        source_candidate_id: Some(id),
    });
    write_index(app_data, &index)?;

    let path = candidate.document_path.map(PathBuf::from);
    let view = store.restore(doc, path);
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
    let _file_io = lock_autosave_files();
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
            let store = app.state::<Mutex<DocumentStore>>();
            if let Err(e) = run_after_interval(store.inner(), &app_data, std::thread::sleep) {
                eprintln!("{e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_model::{EdgeKind, EditOp, Paper};

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

    /// 製品loopが29,999msでは発火せず、要求した30,000msの待機後だけ実行する。
    /// blocking fake sleeperで仮想時間を2回に分け、境界の前後を同じworkerで確認する。
    #[test]
    fn autosave_worker_waits_thirty_seconds_and_still_skips_clean_documents() {
        assert!(Duration::from_millis(29_999) < INTERVAL);
        assert_eq!(INTERVAL, Duration::from_millis(30_000));

        let dirty_dir = temp_dir("interval_dirty");
        let dirty = store_with_edit();
        let (advance_tx, advance_rx) = std::sync::mpsc::channel::<Duration>();
        let (elapsed_tx, elapsed_rx) = std::sync::mpsc::channel::<Duration>();
        std::thread::scope(|scope| {
            let dirty = &dirty;
            let dirty_dir = &dirty_dir;
            let worker = scope.spawn(move || {
                run_after_interval(dirty, dirty_dir, |requested| {
                    assert_eq!(requested, Duration::from_millis(30_000));
                    let mut elapsed = Duration::ZERO;
                    while elapsed < requested {
                        elapsed += advance_rx.recv().expect("仮想時間を受け取る");
                        elapsed_tx.send(elapsed).expect("経過時間を通知する");
                    }
                })
                .unwrap()
            });

            advance_tx.send(Duration::from_millis(29_999)).unwrap();
            assert_eq!(elapsed_rx.recv().unwrap(), Duration::from_millis(29_999));
            assert!(!active_path(dirty_dir).exists(), "29,999msでは0件");

            advance_tx.send(Duration::from_millis(1)).unwrap();
            assert_eq!(elapsed_rx.recv().unwrap(), Duration::from_millis(30_000));
            assert!(worker.join().unwrap(), "30,000ms後はdirty作品を1回書く");
        });
        assert!(active_path(&dirty_dir).is_file());

        let clean_dir = temp_dir("interval_clean");
        let clean = Mutex::new(DocumentStore::default());
        let clean_wrote = run_after_interval(&clean, &clean_dir, |duration| {
            assert_eq!(duration, Duration::from_millis(30_000));
            assert!(!active_path(&clean_dir).exists(), "待機中は0件");
        })
        .unwrap();
        assert!(!clean_wrote, "clean作品は30,000ms後も書かない");
        assert!(!active_path(&clean_dir).exists());
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
        let expected_current = current.lock().unwrap().saved_document();
        assert!(
            !active_path(&dir).exists(),
            "30秒未満の編集なので、まだ現行作業枠へ書かれていない"
        );
        let id = check_all(&dir).unwrap().choices[0]
            .candidate_id
            .expect("前回の候補には番号がある");

        restore_candidate(&current, &dir, Some(id))
            .unwrap()
            .expect("前回の候補を復元できる");

        let choices = check_all(&dir).unwrap().choices;
        assert_eq!(choices.len(), 1, "今の作業を別の候補として残す");
        assert_eq!(
            choices[0].document_path,
            Some(dir.join("いまの水風船.ori3").to_string_lossy().into_owned())
        );
        let carried = std::fs::read_to_string(&choices[0].autosave_path).unwrap();
        assert_eq!(
            parse_document(&carried).unwrap(),
            expected_current,
            "復元直前までの編集内容を同じ製品経路で持ち越す"
        );
    }

    /// 現版が読めない既存候補は将来schemaかもしれないため、壊れたactiveで上書きしない。
    #[test]
    fn unreadable_source_candidate_and_active_are_kept_as_separate_payloads() {
        let dir = temp_dir("unreadable_source_and_active");
        let previous = named_store_with_pending_edit(&dir, "鳥の基本形.ori3");
        run_once(&previous, &dir).unwrap();
        prepare_session(&dir).unwrap();
        let source = check_all(&dir).unwrap().choices[0].clone();
        let source_id = source.candidate_id.expect("元候補IDがある");
        let store = Mutex::new(DocumentStore::default());
        restore_candidate(&store, &dir, Some(source_id))
            .unwrap()
            .expect("正常な元候補を復元できる");

        let future_schema = br#"{"schema_version":999,"future_document":{}}"#;
        let broken_active = br#"{"schema_version":1,"paper":"#;
        std::fs::write(&source.autosave_path, future_schema).unwrap();
        std::fs::write(active_path(&dir), broken_active).unwrap();
        prepare_session(&dir).unwrap();

        assert_eq!(
            std::fs::read(&source.autosave_path).unwrap(),
            future_schema,
            "読めない既存候補を別の読めないactiveで上書きしない"
        );
        let choices = check_all(&dir).unwrap().choices;
        assert_eq!(choices.len(), 2, "既存候補とactiveを個別に保持する");
        assert!(
            choices
                .iter()
                .any(|choice| choice.candidate_id == Some(source_id))
        );
        assert!(choices.iter().any(|choice| {
            choice.candidate_id != Some(source_id)
                && std::fs::read(&choice.autosave_path).ok().as_deref() == Some(broken_active)
        }));
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

    /// 復元した作品を保存しないまま別作品へ切り替え、その別作品を保存しても、
    /// 復元元の候補は利用者が明示的に保存・破棄するまで残す。
    #[test]
    fn switching_document_after_restore_keeps_the_original_candidate() {
        let dir = temp_dir("switch_after_restore");
        let previous = named_store_with_pending_edit(&dir, "折り鶴.ori3");
        run_once(&previous, &dir).unwrap();
        prepare_session(&dir).unwrap();
        let before = check_all(&dir).unwrap().choices;
        assert_eq!(before.len(), 1);
        let source = before[0].clone();
        let source_id = source.candidate_id.expect("持ち越し候補には番号がある");
        let source_payload = std::fs::read(&source.autosave_path).unwrap();

        let restored = Mutex::new(DocumentStore::default());
        restore_candidate(&restored, &dir, Some(source_id))
            .unwrap()
            .expect("折り鶴を復元できる");

        // 製品の document_new → document_save と同じ共有helperで別作品へ切り替える。
        preserve_before_document_change(&restored, &dir).unwrap();
        restored
            .lock()
            .unwrap()
            .new_document(Paper {
                width_mm: 150.0,
                height_mm: 150.0,
            })
            .unwrap();
        let other_path = dir.join("水風船.ori3");
        restored.lock().unwrap().save(Some(&other_path)).unwrap();
        discard_after_save(&dir, Some(&other_path));

        let after = check_all(&dir).unwrap().choices;
        assert_eq!(after.len(), 1, "別作品の保存で復元元を消してはいけない");
        assert_eq!(after[0].candidate_id, Some(source_id));
        assert_eq!(
            std::fs::read(&source.autosave_path).unwrap(),
            source_payload,
            "別作品で復元元のpayloadを上書きしてはいけない"
        );

        // 別作品を編集して次の自動保存・再起動相当を通しても、元候補は別物のまま残る。
        restored
            .lock()
            .unwrap()
            .apply_edit(EditOp::AddSegment {
                a: [0.0, 1.0],
                b: [1.0, 0.0],
                kind: EdgeKind::Valley,
            })
            .unwrap();
        assert!(run_once(&restored, &dir).unwrap());
        prepare_session(&dir).unwrap();
        let after_restart = check_all(&dir).unwrap().choices;
        assert_eq!(after_restart.len(), 2, "元候補と別作品を両方残す");
        assert!(
            after_restart
                .iter()
                .any(|choice| choice.candidate_id == Some(source_id)),
            "復元元の折り鶴を選べる"
        );
        assert_eq!(
            std::fs::read(&source.autosave_path).unwrap(),
            source_payload,
            "次の自動保存でも復元元を上書きしてはいけない"
        );
    }

    const PROCESS_ROLE_ENV: &str = "ORI3_TEST_AUTOSAVE_PROCESS_ROLE";
    const PROCESS_APP_DATA_ENV: &str = "ORI3_TEST_AUTOSAVE_PROCESS_APP_DATA";
    const PROCESS_READY_ENV: &str = "ORI3_TEST_AUTOSAVE_PROCESS_READY";
    const PROCESS_NORMAL_EXIT_ENV: &str = "ORI3_TEST_AUTOSAVE_PROCESS_NORMAL_EXIT";
    const PROCESS_EXPECTED_ENV: &str = "ORI3_TEST_AUTOSAVE_PROCESS_EXPECTED";
    const PROCESS_RESULT_ENV: &str = "ORI3_TEST_AUTOSAVE_PROCESS_RESULT";

    struct NormalExitMarker(PathBuf);

    impl Drop for NormalExitMarker {
        fn drop(&mut self) {
            std::fs::write(&self.0, b"normal exit").ok();
        }
    }

    struct KillOnDropChild(std::process::Child);

    impl Drop for KillOnDropChild {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                self.0.kill().ok();
                self.0.wait().ok();
            }
        }
    }

    fn process_env_path(name: &str) -> PathBuf {
        std::env::var_os(name)
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("{name}が必要"))
    }

    fn process_store(app_data: &Path, name: &str, revision: usize) -> Mutex<DocumentStore> {
        let store = named_store_with_pending_edit(app_data, name);
        let additional = [
            ([0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain),
            ([0.5, 0.0], [0.5, 1.0], EdgeKind::Valley),
            ([0.0, 0.25], [1.0, 0.25], EdgeKind::Mountain),
        ];
        for (a, b, kind) in additional.into_iter().take(revision) {
            store
                .lock()
                .unwrap()
                .apply_edit(EditOp::AddSegment { a, b, kind })
                .unwrap();
        }
        store
    }

    fn saved_document_bytes(store: &Mutex<DocumentStore>) -> Vec<u8> {
        serde_json::to_vec(&store.lock().unwrap().saved_document()).unwrap()
    }

    fn write_expected(store: &Mutex<DocumentStore>) {
        std::fs::write(
            process_env_path(PROCESS_EXPECTED_ENV),
            saved_document_bytes(store),
        )
        .unwrap();
    }

    fn mark_process_result() {
        std::fs::write(process_env_path(PROCESS_RESULT_ENV), b"checked").unwrap();
    }

    fn pause_process_until_killed() -> ! {
        std::fs::write(
            process_env_path(PROCESS_READY_ENV),
            std::process::id().to_string(),
        )
        .unwrap();
        loop {
            std::thread::park_timeout(Duration::from_millis(10));
        }
    }

    fn assert_restored_document(
        store: &Mutex<DocumentStore>,
        expected_path: &Path,
        expected_document_path: &Path,
    ) {
        assert_eq!(
            saved_document_bytes(store),
            std::fs::read(expected_path).unwrap(),
            "別processで復元した作品が強制終了前と一致する"
        );
        let store = store.lock().unwrap();
        assert!(store.is_dirty(), "復元後は明示保存前なのでdirty");
        assert_eq!(
            store.current_path(),
            Some(expected_document_path.to_path_buf()),
            "元の保存先もpayload内のmetadataから戻る"
        );
    }

    fn recovery_info_with_bytes<'a>(
        choices: &'a [RecoveryInfo],
        expected: &[u8],
    ) -> &'a RecoveryInfo {
        choices
            .iter()
            .find(|choice| {
                std::fs::read_to_string(&choice.autosave_path)
                    .ok()
                    .and_then(|text| parse_document(&text).ok())
                    .and_then(|saved| serde_json::to_vec(&saved).ok())
                    .is_some_and(|bytes| bytes == expected)
            })
            .expect("期待した作品内容の候補がある")
    }

    /// 親testが同じlibtest実行物を別OS processとして起動するためのworker。
    /// role未指定の通常test実行では即returnし、再帰的にprocessを増やさない。
    #[test]
    fn autosave_process_contract_child() {
        let Some(role) = std::env::var_os(PROCESS_ROLE_ENV) else {
            return;
        };
        let role = role.to_string_lossy();
        let app_data = process_env_path(PROCESS_APP_DATA_ENV);

        match role.as_ref() {
            "write-complete" => {
                let _normal_exit = NormalExitMarker(process_env_path(PROCESS_NORMAL_EXIT_ENV));
                let store = process_store(&app_data, "折り鶴.ori3", 1);
                write_expected(&store);
                assert!(run_once(&store, &app_data).unwrap());
                pause_process_until_killed();
            }
            "write-partial-index" => {
                let _normal_exit = NormalExitMarker(process_env_path(PROCESS_NORMAL_EXIT_ENV));
                let store = process_store(&app_data, "折り鶴.ori3", 2);
                write_expected(&store);
                let _ = run_once(&store, &app_data);
                panic!("indexのpartial write checkpointで停止しなかった");
            }
            "write-partial-active" => {
                let _normal_exit = NormalExitMarker(process_env_path(PROCESS_NORMAL_EXIT_ENV));
                let store = process_store(&app_data, "折り鶴.ori3", 3);
                let _ = run_once(&store, &app_data);
                panic!("activeのpartial write checkpointで停止しなかった");
            }
            "write-corrupt-payload" => {
                use std::io::Write;

                let _normal_exit = NormalExitMarker(process_env_path(PROCESS_NORMAL_EXIT_ENV));
                let water_path = app_data.join("水風船.ori3");
                let water_id = check_all(&app_data)
                    .unwrap()
                    .choices
                    .iter()
                    .find(|choice| {
                        choice.document_path.as_deref()
                            == Some(water_path.to_string_lossy().as_ref())
                    })
                    .and_then(|choice| choice.candidate_id)
                    .expect("水風船の候補がある");
                let store = Mutex::new(DocumentStore::default());
                restore_candidate(&store, &app_data, Some(water_id))
                    .unwrap()
                    .expect("正常な水風船を復元してactive sourceにできる");
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(active_path(&app_data))
                    .unwrap();
                file.write_all(b"{\"schema_version\":1,\"paper\":").unwrap();
                file.sync_all().unwrap();
                pause_process_until_killed();
            }
            "write-corrupt-index" => {
                use std::io::Write;

                let _normal_exit = NormalExitMarker(process_env_path(PROCESS_NORMAL_EXIT_ENV));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(index_path(&app_data))
                    .unwrap();
                file.write_all(b"{\"version\":1,\"next_candidate_id\":")
                    .unwrap();
                file.sync_all().unwrap();
                pause_process_until_killed();
            }
            "restore-switch-gap" => {
                let _normal_exit = NormalExitMarker(process_env_path(PROCESS_NORMAL_EXIT_ENV));
                let before = check_all(&app_data).unwrap().choices;
                let water_path = app_data.join("水風船.ori3");
                let water_id = before
                    .iter()
                    .find(|choice| {
                        choice.document_path.as_deref()
                            == Some(water_path.to_string_lossy().as_ref())
                    })
                    .and_then(|choice| choice.candidate_id)
                    .expect("水風船の候補がある");
                let store = Mutex::new(DocumentStore::default());
                restore_candidate(&store, &app_data, Some(water_id))
                    .unwrap()
                    .expect("先に水風船を復元できる");
                store
                    .lock()
                    .unwrap()
                    .apply_edit(EditOp::AddSegment {
                        a: [0.25, 0.0],
                        b: [0.25, 1.0],
                        kind: EdgeKind::Valley,
                    })
                    .unwrap();
                std::fs::write(
                    app_data.join("expected-water-after-edit.json"),
                    saved_document_bytes(&store),
                )
                .unwrap();

                let crane_path = app_data.join("折り鶴.ori3");
                let crane_id = check_all(&app_data)
                    .unwrap()
                    .choices
                    .iter()
                    .find(|choice| {
                        choice.document_path.as_deref()
                            == Some(crane_path.to_string_lossy().as_ref())
                    })
                    .and_then(|choice| choice.candidate_id)
                    .expect("折り鶴の候補がある");
                arm_pause_after_restore_active_write();
                let _ = restore_candidate(&store, &app_data, Some(crane_id));
                panic!("復元payload/index境界のcheckpointで停止しなかった");
            }
            "read-exact" => {
                prepare_session(&app_data).unwrap();
                let choices = check_all(&app_data).unwrap().choices;
                assert_eq!(choices.len(), 1, "強制終了した1作品を候補として返す");
                let info = &choices[0];
                let candidate_id = info.candidate_id.expect("持ち越し候補IDがある");
                let expected_document_path = app_data.join("折り鶴.ori3");
                assert_eq!(
                    info.document_path.as_deref(),
                    Some(expected_document_path.to_string_lossy().as_ref()),
                    "index確定前でも作品名をpayloadから戻す"
                );
                assert!(info.saved_at_ms.is_some());
                assert_eq!(
                    info.step_count,
                    payload_step_count(Path::new(&info.autosave_path)).unwrap()
                );
                let store = Mutex::new(DocumentStore::default());
                restore_candidate(&store, &app_data, Some(candidate_id))
                    .unwrap()
                    .expect("別processで復元できる");
                assert_restored_document(
                    &store,
                    &process_env_path(PROCESS_EXPECTED_ENV),
                    &expected_document_path,
                );
                assert!(
                    Path::new(&info.autosave_path).is_file(),
                    "明示保存前なので復元元候補を保持する"
                );
                mark_process_result();
            }
            "read-restore-switch-gap" => {
                prepare_session(&app_data).unwrap();
                let choices = check_all(&app_data).unwrap().choices;
                assert_eq!(
                    choices.len(),
                    2,
                    "復元切替中に止まっても折り鶴と編集中の水風船を各1件残す"
                );
                let crane_expected = std::fs::read(process_env_path(PROCESS_EXPECTED_ENV)).unwrap();
                let water_expected =
                    std::fs::read(app_data.join("expected-water-after-edit.json")).unwrap();
                let crane_id = recovery_info_with_bytes(&choices, &crane_expected)
                    .candidate_id
                    .expect("折り鶴の候補IDがある");
                let water_id = recovery_info_with_bytes(&choices, &water_expected)
                    .candidate_id
                    .expect("水風船の候補IDがある");

                let store = Mutex::new(DocumentStore::default());
                restore_candidate(&store, &app_data, Some(crane_id))
                    .unwrap()
                    .expect("折り鶴を別processで復元できる");
                assert_restored_document(
                    &store,
                    &process_env_path(PROCESS_EXPECTED_ENV),
                    &app_data.join("折り鶴.ori3"),
                );
                restore_candidate(&store, &app_data, Some(water_id))
                    .unwrap()
                    .expect("直前まで編集中だった水風船も別processで復元できる");
                assert_restored_document(
                    &store,
                    &app_data.join("expected-water-after-edit.json"),
                    &app_data.join("水風船.ori3"),
                );
                assert!(
                    check_all(&app_data)
                        .unwrap()
                        .choices
                        .iter()
                        .any(|choice| choice.candidate_id == Some(crane_id)),
                    "最後に選ばなかった折り鶴も明示保存までは残す"
                );
                mark_process_result();
            }
            "read-corrupt-payload" => {
                prepare_session(&app_data).unwrap();
                let choices = check_all(&app_data).unwrap().choices;
                assert_eq!(
                    choices.len(),
                    3,
                    "正常な折り鶴・復元元の水風船・壊れたactiveを個別に扱う"
                );
                let corrupt = choices
                    .iter()
                    .find(|choice| {
                        std::fs::read_to_string(&choice.autosave_path)
                            .ok()
                            .is_none_or(|text| parse_document(&text).is_err())
                    })
                    .expect("壊れた候補も黙って削除せず返す");
                assert_eq!(corrupt.step_count, 0);
                let corrupt_id = corrupt.candidate_id.unwrap();
                let corrupt_path = PathBuf::from(&corrupt.autosave_path);
                let store = Mutex::new(DocumentStore::default());
                assert!(
                    restore_candidate(&store, &app_data, Some(corrupt_id)).is_err(),
                    "壊れた1件だけを復元errorにする"
                );
                assert!(
                    check_all(&app_data)
                        .unwrap()
                        .choices
                        .iter()
                        .any(|choice| choice.candidate_id == Some(corrupt_id)),
                    "復元errorだけでは候補を消さない"
                );

                let crane_expected = std::fs::read(process_env_path(PROCESS_EXPECTED_ENV)).unwrap();
                let water_expected = std::fs::read(app_data.join("expected-water.json")).unwrap();
                let crane_id = recovery_info_with_bytes(&choices, &crane_expected)
                    .candidate_id
                    .unwrap();
                let water_id = recovery_info_with_bytes(&choices, &water_expected)
                    .candidate_id
                    .unwrap();
                restore_candidate(&store, &app_data, Some(crane_id))
                    .unwrap()
                    .expect("壊れた候補があっても折り鶴を復元できる");
                assert_restored_document(
                    &store,
                    &process_env_path(PROCESS_EXPECTED_ENV),
                    &app_data.join("折り鶴.ori3"),
                );
                restore_candidate(&store, &app_data, Some(water_id))
                    .unwrap()
                    .expect("壊れたactiveで元候補を上書きせず水風船を復元できる");
                assert_restored_document(
                    &store,
                    &app_data.join("expected-water.json"),
                    &app_data.join("水風船.ori3"),
                );
                assert!(discard_candidate(&app_data, Some(corrupt_id)).unwrap());
                assert!(!corrupt_path.exists(), "選んだ壊れた候補だけを破棄する");
                assert!(run_once(&store, &app_data).unwrap(), "自動保存も続行できる");
                mark_process_result();
            }
            "read-rebuilt-index" => {
                prepare_session(&app_data).unwrap();
                let choices = check_all(&app_data).unwrap().choices;
                assert_eq!(choices.len(), 2, "壊れた索引から2作品を再構築する");
                let crane_expected = std::fs::read(process_env_path(PROCESS_EXPECTED_ENV)).unwrap();
                let water_expected = std::fs::read(app_data.join("expected-water.json")).unwrap();
                let crane = recovery_info_with_bytes(&choices, &crane_expected);
                let water = recovery_info_with_bytes(&choices, &water_expected);
                assert_eq!(
                    crane.document_path.as_deref(),
                    Some(app_data.join("折り鶴.ori3").to_string_lossy().as_ref())
                );
                assert_eq!(
                    water.document_path.as_deref(),
                    Some(app_data.join("水風船.ori3").to_string_lossy().as_ref())
                );
                let water_id = water.candidate_id.unwrap();
                let store = Mutex::new(DocumentStore::default());
                restore_candidate(&store, &app_data, Some(water_id))
                    .unwrap()
                    .expect("再構築した現在作品を復元できる");
                assert_restored_document(
                    &store,
                    &app_data.join("expected-water.json"),
                    &app_data.join("水風船.ori3"),
                );
                assert!(
                    check_all(&app_data)
                        .unwrap()
                        .choices
                        .iter()
                        .any(|choice| choice.candidate_id == crane.candidate_id),
                    "選ばなかった正常候補を保持する"
                );
                assert!(app_data.read_dir().unwrap().any(|entry| {
                    entry
                        .ok()
                        .and_then(|entry| entry.file_name().to_str().map(str::to_owned))
                        .is_some_and(|name| name.ends_with(".corrupt"))
                }));
                assert!(
                    run_once(&store, &app_data).unwrap(),
                    "修復後も自動保存できる"
                );
                mark_process_result();
            }
            other => panic!("不明な強制終了検査role: {other}"),
        }
    }

    fn process_test_command(role: &str, app_data: &Path) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("autosave::tests::autosave_process_contract_child")
            .arg("--exact")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(PROCESS_ROLE_ENV, role)
            .env(PROCESS_APP_DATA_ENV, app_data)
            .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
    }

    fn wait_until_ready(child: &mut KillOnDropChild, ready: &Path) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if ready.is_file() {
                return;
            }
            if let Some(status) = child.0.try_wait().unwrap() {
                panic!(
                    "ready前に子processが終了した: pid={}, status={status}",
                    child.0.id()
                );
            }
            if std::time::Instant::now() >= deadline {
                child.0.kill().ok();
                let status = child.0.wait().unwrap();
                panic!(
                    "子processのready待ちが10秒を超えた: pid={}, status={status}",
                    child.0.id()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn force_kill_and_wait(
        child: &mut KillOnDropChild,
        ready: &Path,
        normal_exit: &Path,
        phase: &str,
    ) -> u32 {
        wait_until_ready(child, ready);
        let pid = child.0.id();
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        println!(
            "forced-kill phase={phase} pid={pid} status={status} code={:?}",
            status.code()
        );
        assert!(!status.success(), "強制終了processは成功終了ではない");
        assert!(
            !normal_exit.exists(),
            "Dropによる正常終了markerを通ってはいけない"
        );
        pid
    }

    fn run_recovery_process(
        role: &str,
        app_data: &Path,
        expected: &Path,
        result: &Path,
        killed_pid: u32,
    ) {
        let mut command = process_test_command(role, app_data);
        command
            .env(PROCESS_EXPECTED_ENV, expected)
            .env(PROCESS_RESULT_ENV, result)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut child = KillOnDropChild(command.spawn().unwrap());
        let recovery_pid = child.0.id();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                child.0.kill().ok();
                let status = child.0.wait().unwrap();
                panic!(
                    "復旧processが10秒以内に終了しなかった: role={role}, pid={recovery_pid}, status={status}"
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        println!(
            "restart role={role} pid={recovery_pid} status={} code={:?}",
            status,
            status.code()
        );
        assert_ne!(recovery_pid, killed_pid, "別PIDで再起動する");
        assert_eq!(status.code(), Some(0), "新processの実終了コードは0");
        assert!(result.is_file(), "reader本体が最後まで契約を検査した");
    }

    fn partial_temp_files(app_data: &Path, target_name: &str) -> Vec<PathBuf> {
        let prefix = format!(".{target_name}.");
        app_data
            .read_dir()
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"))
            })
            .collect()
    }

    /// 本丸: 実autosave完了後にOS processを強制終了し、別PIDのprocessで完全復元する。
    #[test]
    fn forced_process_kill_then_new_process_restores_exact_document() {
        let dir = temp_dir("process_kill_complete");
        let ready = dir.join("writer.ready");
        let normal_exit = dir.join("writer.normal-exit");
        let expected = dir.join("expected.json");
        let result = dir.join("reader.result");
        let mut command = process_test_command("write-complete", &dir);
        command
            .env(PROCESS_READY_ENV, &ready)
            .env(PROCESS_NORMAL_EXIT_ENV, &normal_exit)
            .env(PROCESS_EXPECTED_ENV, &expected)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut writer = KillOnDropChild(command.spawn().unwrap());
        let killed_pid =
            force_kill_and_wait(&mut writer, &ready, &normal_exit, "completed-autosave");
        assert!(active_path(&dir).is_file());
        assert!(index_path(&dir).is_file());
        run_recovery_process("read-exact", &dir, &expected, &result, killed_pid);
    }

    /// 復元候補Bを編集中に候補Aへ切り替え、A payload確定後・index確定前でkillする。
    /// 再起動後もAとBを取り違えず、30秒未満だったBの最終編集まで両方復元する。
    #[test]
    fn forced_kill_during_candidate_restore_keeps_both_exact_documents() {
        let dir = temp_dir("process_kill_restore_switch_gap");
        let expected = dir.join("expected-crane.json");
        let crane = process_store(&dir, "折り鶴.ori3", 0);
        std::fs::write(&expected, saved_document_bytes(&crane)).unwrap();
        assert!(run_once(&crane, &dir).unwrap());
        prepare_session(&dir).unwrap();
        let water = process_store(&dir, "水風船.ori3", 1);
        assert!(run_once(&water, &dir).unwrap());
        prepare_session(&dir).unwrap();
        assert_eq!(check_all(&dir).unwrap().choices.len(), 2);

        let ready = dir.join("writer.ready");
        let normal_exit = dir.join("writer.normal-exit");
        let result = dir.join("reader.result");
        let mut command = process_test_command("restore-switch-gap", &dir);
        command
            .env(PROCESS_READY_ENV, &ready)
            .env(PROCESS_NORMAL_EXIT_ENV, &normal_exit)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut writer = KillOnDropChild(command.spawn().unwrap());
        let killed_pid = force_kill_and_wait(
            &mut writer,
            &ready,
            &normal_exit,
            "restore-payload-before-index",
        );

        let index = read_index(&dir).unwrap().expect("第1段のindexは確定済み");
        assert!(
            index.active.is_none(),
            "旧sourceとの結び付きを外したindexを先に確定する"
        );
        let active = std::fs::read_to_string(active_path(&dir)).unwrap();
        assert_eq!(
            serde_json::to_vec(&parse_document(&active).unwrap()).unwrap(),
            std::fs::read(&expected).unwrap(),
            "選んだ折り鶴payloadは完成済み"
        );
        assert!(
            dir.join("expected-water-after-edit.json").is_file(),
            "kill直前の水風船の内容を別processが記録した"
        );
        run_recovery_process(
            "read-restore-switch-gap",
            &dir,
            &expected,
            &result,
            killed_pid,
        );
    }

    /// index tempの半分を書いた実processをkillし、完成済みactiveから索引を再構築する。
    #[test]
    fn forced_kill_during_index_write_recovers_payload_and_metadata() {
        let dir = temp_dir("process_kill_partial_index");
        let ready = dir.join("writer.ready");
        let normal_exit = dir.join("writer.normal-exit");
        let expected = dir.join("expected.json");
        let result = dir.join("reader.result");
        let mut command = process_test_command("write-partial-index", &dir);
        command
            .env(PROCESS_NORMAL_EXIT_ENV, &normal_exit)
            .env(PROCESS_EXPECTED_ENV, &expected)
            .env("ORI3_TEST_PAUSE_PARTIAL_ATOMIC_TARGET", index_path(&dir))
            .env("ORI3_TEST_PAUSE_PARTIAL_ATOMIC_READY", &ready)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut writer = KillOnDropChild(command.spawn().unwrap());
        let killed_pid = force_kill_and_wait(&mut writer, &ready, &normal_exit, "partial-index");
        assert!(active_path(&dir).is_file(), "payload本体はatomic確定済み");
        assert!(!index_path(&dir).exists(), "最初の索引はまだ確定していない");
        let partial = partial_temp_files(&dir, INDEX_FILE);
        assert_eq!(partial.len(), 1, "書きかけのindex tempが実際に残る");
        let partial_bytes = std::fs::read(&partial[0]).unwrap();
        assert!(!partial_bytes.is_empty());
        assert!(serde_json::from_slice::<AutosaveIndex>(&partial_bytes).is_err());
        run_recovery_process("read-exact", &dir, &expected, &result, killed_pid);
    }

    /// active tempの半分でkillしても、旧active/indexの完成snapshotを復元する。
    #[test]
    fn forced_kill_during_payload_write_keeps_last_complete_snapshot() {
        let dir = temp_dir("process_kill_partial_active");
        let expected = dir.join("expected.json");
        let baseline = process_store(&dir, "折り鶴.ori3", 0);
        std::fs::write(&expected, saved_document_bytes(&baseline)).unwrap();
        assert!(run_once(&baseline, &dir).unwrap());
        let active_before = std::fs::read(active_path(&dir)).unwrap();
        let index_before = std::fs::read(index_path(&dir)).unwrap();

        let ready = dir.join("writer.ready");
        let normal_exit = dir.join("writer.normal-exit");
        let result = dir.join("reader.result");
        let mut command = process_test_command("write-partial-active", &dir);
        command
            .env(PROCESS_NORMAL_EXIT_ENV, &normal_exit)
            .env("ORI3_TEST_PAUSE_PARTIAL_ATOMIC_TARGET", active_path(&dir))
            .env("ORI3_TEST_PAUSE_PARTIAL_ATOMIC_READY", &ready)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut writer = KillOnDropChild(command.spawn().unwrap());
        let killed_pid = force_kill_and_wait(&mut writer, &ready, &normal_exit, "partial-payload");
        assert_eq!(std::fs::read(active_path(&dir)).unwrap(), active_before);
        assert_eq!(std::fs::read(index_path(&dir)).unwrap(), index_before);
        let partial = partial_temp_files(&dir, CURRENT_FILE);
        assert_eq!(partial.len(), 1, "書きかけのpayload tempが実際に残る");
        assert!(
            std::fs::read_to_string(&partial[0])
                .ok()
                .is_none_or(|text| parse_document(&text).is_err()),
            "partial tempを完成作品として読めない"
        );
        run_recovery_process("read-exact", &dir, &expected, &result, killed_pid);
    }

    /// 壊れたpayloadは1候補だけerrorにし、別候補・起動・自動保存を止めない。
    #[test]
    fn corrupt_payload_after_forced_kill_does_not_block_other_recovery() {
        let dir = temp_dir("process_kill_corrupt_payload");
        let expected = dir.join("expected.json");
        let valid = process_store(&dir, "折り鶴.ori3", 0);
        std::fs::write(&expected, saved_document_bytes(&valid)).unwrap();
        assert!(run_once(&valid, &dir).unwrap());
        prepare_session(&dir).unwrap();
        let water = process_store(&dir, "水風船.ori3", 1);
        std::fs::write(
            dir.join("expected-water.json"),
            saved_document_bytes(&water),
        )
        .unwrap();
        assert!(run_once(&water, &dir).unwrap());
        prepare_session(&dir).unwrap();

        let ready = dir.join("writer.ready");
        let normal_exit = dir.join("writer.normal-exit");
        let result = dir.join("reader.result");
        let mut command = process_test_command("write-corrupt-payload", &dir);
        command
            .env(PROCESS_READY_ENV, &ready)
            .env(PROCESS_NORMAL_EXIT_ENV, &normal_exit)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut writer = KillOnDropChild(command.spawn().unwrap());
        let killed_pid = force_kill_and_wait(&mut writer, &ready, &normal_exit, "corrupt-payload");
        run_recovery_process("read-corrupt-payload", &dir, &expected, &result, killed_pid);
    }

    /// 壊れたindexを退避し、正常payload群から全候補を再構築して動作を続ける。
    #[test]
    fn corrupt_index_after_forced_kill_is_backed_up_and_rebuilt() {
        let dir = temp_dir("process_kill_corrupt_index");
        let expected = dir.join("expected.json");
        let crane = process_store(&dir, "折り鶴.ori3", 0);
        std::fs::write(&expected, saved_document_bytes(&crane)).unwrap();
        assert!(run_once(&crane, &dir).unwrap());
        prepare_session(&dir).unwrap();
        let water = process_store(&dir, "水風船.ori3", 2);
        std::fs::write(
            dir.join("expected-water.json"),
            saved_document_bytes(&water),
        )
        .unwrap();
        assert!(run_once(&water, &dir).unwrap());

        let ready = dir.join("writer.ready");
        let normal_exit = dir.join("writer.normal-exit");
        let result = dir.join("reader.result");
        let mut command = process_test_command("write-corrupt-index", &dir);
        command
            .env(PROCESS_READY_ENV, &ready)
            .env(PROCESS_NORMAL_EXIT_ENV, &normal_exit)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut writer = KillOnDropChild(command.spawn().unwrap());
        let killed_pid = force_kill_and_wait(&mut writer, &ready, &normal_exit, "corrupt-index");
        run_recovery_process("read-rebuilt-index", &dir, &expected, &result, killed_pid);
    }
}
