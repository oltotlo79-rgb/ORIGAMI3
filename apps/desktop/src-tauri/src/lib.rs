pub mod autosave;
pub mod commands;
pub mod store;

/// テスト専用: 実際にアロケータへ確保された正味バイト数を、**スレッドごとに**数える。
///
/// SYS-002(取り消し履歴の記憶使用量)の実測の裏取りに使う。`System` アロケータへ
/// 全て委譲するだけで、確保・解放の成否や整列は一切変えない。本番ビルドには
/// 含めない(`cfg(test)`)ため、配布物の挙動には影響しない。
///
/// **なぜスレッドごとに分けるか**: 以前はプロセスに1個の `AtomicI64` で数えていた。
/// `cargo test` は既定でテストごとに別スレッドを立てて同時に走らせるため、
/// 測定区間の間に**他のテストが確保した分**が同じカウンタへ混入し、
/// 同じ検査が単独実行では800,384バイト、並列実行では58,315,019〜78,370,317バイト
/// (実行のたびに変わる)という65〜98倍の食い違いを起こしていた。
/// スレッドごとに分ければ、同時に走る他のテストは別のカウンタを触るので混ざらない
/// (実測は `scratchpad/undo-memory-fix-report.md` §段階1)。
///
/// `Cell<i64>` は `const` 初期化で、後始末(`Drop`)も遅延初期化も伴わないため、
/// この参照自体が新たな確保を起こすことはない(アロケータの中から触っても安全)。
///
/// (clippyの`items_after_test_module`を避けるため、`#[cfg(test)] mod
/// surface_order_acceptance;`より前に置く)
#[cfg(test)]
pub(crate) mod alloc_probe {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        /// このスレッドが確保して、まだ解放していない正味バイト数。
        static CURRENT_BYTES: Cell<i64> = const { Cell::new(0) };
    }

    /// いま呼んでいるスレッドの正味確保量(バイト)。
    ///
    /// スレッドの後始末中など、スレッド局所変数へ触れない状況では `0` を返す
    /// (測定はテスト本体の実行中にしか行わないので、実際には起こらない)。
    pub fn current_thread_bytes() -> i64 {
        CURRENT_BYTES.try_with(Cell::get).unwrap_or(0)
    }

    fn add(delta: i64) {
        let _ = CURRENT_BYTES.try_with(|bytes| bytes.set(bytes.get() + delta));
    }

    pub struct CountingAllocator;

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                add(layout.size() as i64);
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) };
            add(-(layout.size() as i64));
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;
}

#[cfg(test)]
mod surface_order_acceptance;

use std::sync::Mutex;

use tauri::Manager;

use commands::ProposalJobs;
use store::DocumentStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(DocumentStore::default()))
        .manage(ProposalJobs::default())
        .invoke_handler(tauri::generate_handler![
            commands::document_new,
            commands::document_open,
            commands::document_save,
            commands::edit_apply,
            commands::edit_apply_batch,
            commands::edit_undo,
            commands::edit_redo,
            commands::sequence_apply,
            commands::sequence_replay,
            commands::pose_solve,
            commands::fold_all_preview,
            commands::recovery_check,
            commands::recovery_restore,
            commands::proposal_generate_job,
            commands::proposal_progress,
            commands::proposal_control,
            commands::proposal_apply,
            commands::document_export,
        ])
        // 30秒ごとの自動保存(SYS-003)。書き出しはこのスレッドの中だけで完結する
        .setup(|app| {
            autosave::spawn(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        // 正常終了なら自動保存は要らない。残っていれば異常終了とみなして復元を提案する
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit)
                && let Ok(dir) = autosave::app_data_dir(app)
            {
                // 未保存の作業は異常終了と同じく次回の復旧対象として残す。
                // 保存済みの場合だけ、正常終了として自動保存を片付ける。
                autosave::discard_if_clean(app.state::<Mutex<DocumentStore>>().inner(), &dir);
            }
        });
}
