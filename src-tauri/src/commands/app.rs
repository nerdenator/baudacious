//! Application-level Tauri commands

/// Exit the application cleanly.
#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}
