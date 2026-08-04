pub mod commands;
pub mod store;

use std::sync::Mutex;

use store::DocumentStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(DocumentStore::default()))
        .invoke_handler(tauri::generate_handler![
            commands::document_new,
            commands::document_open,
            commands::document_save,
            commands::edit_apply,
            commands::edit_undo,
            commands::edit_redo,
            commands::sequence_apply,
            commands::pose_solve,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
