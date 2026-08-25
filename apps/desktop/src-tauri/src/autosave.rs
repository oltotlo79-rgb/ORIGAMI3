//! 自動保存とクラッシュ復旧(SYS-003)。
//!
//! 30秒ごとに、未保存の変更があるときだけ作業中の内容を別ファイルへ書き出す。
//! 書き出し先は保存先が決まっていれば `<保存先>.autosave`、無題ならOSのアプリ
//! データディレクトリ配下の固定名。どこへ書いたかは同ディレクトリの目印ファイルに
//! 残し、次の起動時にそれを見て復元を提案する。
//! 正常終了時と明示保存の成功時には、自動保存ファイルと目印を消す。
//!
//! 設計規約: ロックは複製を取る一瞬だけ。JSON化とファイル書き出しはロックの外。
//! `DocumentStore::save` は使わない(保存先パスと未保存フラグを書き換えるため)。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

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

/// 復元の案内に必要な情報(フロントの復旧ダイアログが使う)
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryInfo {
    /// 自動保存ファイルの場所
    pub autosave_path: String,
    /// 元の保存先(無題だったならNone)
    pub document_path: Option<String>,
    /// 最後に自動保存した時刻(1970年からのミリ秒)。分からなければNone
    pub saved_at_ms: Option<u64>,
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

/// 複製した作品を自動保存ファイルへ書き、場所を目印ファイルへ控える。
fn write_snapshot(
    doc: &SavedDocument,
    doc_path: Option<&Path>,
    app_data: &Path,
) -> Result<(), String> {
    let target = target_path(doc_path, app_data);
    let json = serde_json::to_string_pretty(doc)
        .map_err(|e| format!("自動保存データの作成に失敗しました: {e}"))?;
    std::fs::create_dir_all(app_data).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(&target, json.as_bytes()).map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    write_atomic(
        marker_path(app_data).as_path(),
        target.to_string_lossy().as_bytes(),
    )
    .map_err(|e| format!("自動保存に失敗しました: {e}"))?;
    Ok(())
}

/// 前回の自動保存が残っていれば、その情報を返す(起動時の復旧確認)。
/// 正常終了・明示保存のたびに消しているので、残っていれば異常終了とみなせる。
pub fn check(app_data: &Path) -> Option<RecoveryInfo> {
    let autosave = recorded_autosave_path(app_data, None, true)?;
    let meta = std::fs::metadata(&autosave).ok()?;
    let saved_at_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    Some(RecoveryInfo {
        autosave_path: autosave.to_string_lossy().into_owned(),
        document_path: document_path_of(&autosave).map(|p| p.to_string_lossy().into_owned()),
        saved_at_ms,
    })
}

/// 自動保存ファイルと目印を消す(正常終了時・明示保存の成功時・利用者が破棄したとき)。
/// 消せなくても呼び出し側を止めない(次の自動保存で上書きされる)。
pub fn discard(app_data: &Path, current_document: Option<&Path>) {
    if let Some(autosave) = recorded_autosave_path(app_data, current_document, false) {
        std::fs::remove_file(autosave).ok();
    }
    std::fs::remove_file(marker_path(app_data)).ok();
}

/// 正常終了時だけ自動保存を片付ける。未保存なら次回の復旧に残す。
pub fn discard_if_clean(store: &Mutex<DocumentStore>, app_data: &Path) {
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_dirty() {
        return;
    }
    let path = guard.current_path();
    drop(guard);
    discard(app_data, path.as_deref());
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
    let text = std::fs::read_to_string(&info.autosave_path)
        .map_err(|e| format!("作業中だった内容を読み込めませんでした: {e}"))?;
    let doc = parse_document(&text)?;
    let path = info.document_path.map(PathBuf::from);
    let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
    // 復旧前から開いていた作品だけが、外部の`.autosave`を消してよい保存先になる。
    // 目印から今セットしたpathを信用して削除対象にしてはいけない。
    let previous_path = guard.current_path();
    let view = guard.restore(doc, path);
    drop(guard);
    // 復元した内容は画面に載ったので、同じ提案を次の起動で繰り返さない
    discard(app_data, previous_path.as_deref());
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
        // 書き出し先は<保存先>.autosave、元ファイルは自動保存で上書きされない
        let autosave = dir.join("作品.ori3.autosave");
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
        assert_eq!(
            info.autosave_path,
            dir.join(UNTITLED_FILE).to_string_lossy()
        );
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

    /// 破棄すると自動保存ファイルも目印も消える。
    #[test]
    fn discard_removes_autosave_file() {
        let dir = temp_dir("discard");
        let store = store_with_edit();
        run_once(&store, &dir).unwrap();
        let autosave = PathBuf::from(check(&dir).unwrap().autosave_path);
        assert!(autosave.is_file());

        discard(&dir, None);
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

        discard(&dir, None);

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

    #[test]
    fn test_app_data_dir_override_is_opt_in_and_preserves_the_unset_path() {
        assert_eq!(test_app_data_dir_override(None), None);
        assert_eq!(test_app_data_dir_override(Some(OsString::new())), None);
        assert_eq!(
            test_app_data_dir_override(Some(OsString::from("C:\\隔離\\autosave"))),
            Some(PathBuf::from("C:\\隔離\\autosave"))
        );
    }
}
