//! IPCコマンド層: 各コマンドはDocumentStoreへ委譲するだけの薄いラッパー。
//! 全コマンドをpanic捕捉ラッパー`guard`で包み、アプリを落とさない(SYS-005)。
//! 全コマンドを`#[tauri::command(async)]`にしてスレッドプールで実行する
//! (同期fnはメインスレッド実行になり、validate等の計算でUIが引っかかるため)。
//!
//! 設計規約: ロック中に重い計算をしない(pose_solveなど他コマンドを待たせないため)。
//! ロック下ではstoreの状態更新と複製だけを行い、手順の再生や姿勢計算は
//! ロックを解放してから実行する(`view_command` / `pose_solve` / `sequence_replay`)。

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use tauri::State;

use crate::store::{DocumentStore, DocumentView, add_penetration_warning, attach_replay};
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

/// DocumentViewを返す操作の共通後処理: 手順を最新ステップまで自動再生して
/// 立体・飛ばした手順・警告をビューへ載せる(SEQ-004)。
///
/// 設計規約: ロック中に重い計算をしない。`f` の中で取ったロックは `f` を抜けた時点で
/// 解放されているので、再生(面400・10手順でrelease約23ms)はロックの外で走る。
fn view_command(f: impl FnOnce() -> Result<DocumentView, String>) -> Result<DocumentView, String> {
    let mut view = f()?; // ここでロックは解放済み
    attach_replay(&mut view);
    Ok(view)
}

#[tauri::command(async)]
pub fn document_new(
    state: State<'_, Mutex<DocumentStore>>,
    paper: Paper,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).new_document(paper))
    }))
}

#[tauri::command(async)]
pub fn document_open(
    state: State<'_, Mutex<DocumentStore>>,
    path: String,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).open(Path::new(&path)))
    }))
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
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).apply_edit(op))
    }))
}

#[tauri::command(async)]
pub fn edit_undo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| view_command(|| lock(&state).undo())))
}

#[tauri::command(async)]
pub fn edit_redo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| view_command(|| lock(&state).redo())))
}

#[tauri::command(async)]
pub fn sequence_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: SeqOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).apply_seq(op))
    }))
}

/// 折り角度の追従計算(Task 1-8)。driver角を固定して残りのヒンジ角を解き、
/// 3D表示用フレームを返す。前回解はstoreが保持し、warm startとして使う。
/// facesは編集時に導出済みのstoreのキャッシュを流用する(extract_faces再実行なし)。
///
/// 設計規約: ロック中に重い計算をしない(将来の自動保存スレッドとの共存のため)。
/// ロック下ではCP・faces・前回解の複製だけを行って即ロックを解放し、
/// solveはロックの外で実行し、結果の角度だけを短いロックで書き戻す。
#[tauri::command(async)]
pub fn pose_solve(
    state: State<'_, Mutex<DocumentStore>>,
    drivers: Vec<Driver>,
) -> Result<ori3_rigid::SolveResult, String> {
    guard(AssertUnwindSafe(|| {
        let (cp, faces, warm) = lock(&state).pose_inputs(); // 複製のみ、即ロック解放
        let mut result = ori3_rigid::solve(&cp, &faces, &drivers, warm.as_ref());
        add_penetration_warning(&mut result.frame); // 紙のめり込み(SIM-007)
        lock(&state).store_pose_angles(result.angles.clone()); // 短いロックで書き戻し
        Ok(result)
    }))
}

/// 手順の再生(Task 2-3)。展開図と手順列から `up_to` ステップ目(補間係数 `t`)の
/// 立体を求め直す。3D状態は保存しないので、展開図を編集した後でも再生できる。
///
/// 設計規約: ロック中に重い計算をしない。ロック下ではDocumentと導出済みfacesの複製
/// だけを行って即ロックを解放し、再生はロックの外で実行する(結果の書き戻しは不要)。
#[tauri::command(async)]
pub fn sequence_replay(
    state: State<'_, Mutex<DocumentStore>>,
    up_to: usize,
    t: f64,
) -> Result<ori3_layers::ReplayResult, String> {
    guard(AssertUnwindSafe(|| {
        let (doc, faces) = lock(&state).replay_inputs(); // 複製のみ、即ロック解放
        let mut result = ori3_layers::replay_with_faces(&doc, &faces, up_to, t);
        // 折る途中(t<1)は立体になるので、紙が食い込んでいないかを見る(SIM-007)。
        // 画面のバッジは ReplayResult.warnings を見るので両方へ同じ文言を載せる
        if add_penetration_warning(&mut result.frame) {
            result
                .warnings
                .push(ori3_rigid::PENETRATION_WARNING.to_string());
        }
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
