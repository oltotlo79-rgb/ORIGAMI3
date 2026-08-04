//! IPCコマンド層: 各コマンドはDocumentStoreへ委譲するだけの薄いラッパー。
//! 全コマンドをpanic捕捉ラッパー`guard`で包み、アプリを落とさない(SYS-005)。
//! 全コマンドを`#[tauri::command(async)]`にしてスレッドプールで実行する
//! (同期fnはメインスレッド実行になり、validate等の計算でUIが引っかかるため)。

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use tauri::State;

use crate::store::{DocumentStore, DocumentView};
use ori3_model::{Driver, EditOp, Paper, SeqOp};

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

#[tauri::command(async)]
pub fn document_new(
    state: State<'_, Mutex<DocumentStore>>,
    paper: Paper,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| lock(&state).new_document(paper)))
}

#[tauri::command(async)]
pub fn document_open(
    state: State<'_, Mutex<DocumentStore>>,
    path: String,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| lock(&state).open(Path::new(&path))))
}

#[tauri::command(async)]
pub fn document_save(
    state: State<'_, Mutex<DocumentStore>>,
    path: Option<String>,
) -> Result<(), String> {
    guard(AssertUnwindSafe(|| {
        lock(&state).save(path.as_deref().map(Path::new))
    }))
}

#[tauri::command(async)]
pub fn edit_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: EditOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| lock(&state).apply_edit(op)))
}

#[tauri::command(async)]
pub fn edit_undo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| lock(&state).undo()))
}

#[tauri::command(async)]
pub fn edit_redo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| lock(&state).redo()))
}

#[tauri::command(async)]
pub fn sequence_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: SeqOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| lock(&state).apply_seq(op)))
}

/// 折り角度の追従計算(Task 1-8)。driver角を固定して残りのヒンジ角を解き、
/// 3D表示用フレームを返す。前回解はstoreが保持し、warm startとして使う。
///
/// 設計規約: ロック中に重い計算をしない(将来の自動保存スレッドとの共存のため)。
/// ロック下ではCPの複製と前回解の取得だけを行って即ロックを解放し、
/// solveはロックの外で実行し、結果の角度だけを短いロックで書き戻す。
#[tauri::command(async)]
pub fn pose_solve(
    state: State<'_, Mutex<DocumentStore>>,
    drivers: Vec<Driver>,
) -> Result<ori3_rigid::SolveResult, String> {
    guard(AssertUnwindSafe(|| {
        let (cp, warm) = lock(&state).pose_inputs(); // 複製のみ、即ロック解放
        let faces = ori3_cp::extract_faces(&cp);
        let result = ori3_rigid::solve(&cp, &faces, &drivers, warm.as_ref());
        lock(&state).store_pose_angles(result.angles.clone()); // 短いロックで書き戻し
        Ok(result)
    }))
}

#[cfg(test)]
mod tests {
    use super::guard;
    use std::panic::AssertUnwindSafe;

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
