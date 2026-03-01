//! Status command — returns current connection state for frontend hydration

use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::State;

use crate::state::AppState;

/// Snapshot of runtime connection state, returned by `get_connection_status`.
/// Used by the frontend status bar to reconstruct indicator state after a reload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub serial_connected: bool,
    pub serial_port: Option<String>,
    pub audio_streaming: bool,
    pub audio_device: Option<String>,
}

#[tauri::command]
pub fn get_connection_status(state: State<'_, AppState>) -> ConnectionStatus {
    let serial_connected = state.radio.lock().unwrap().is_some();
    let serial_port = state.serial_port_name.lock().unwrap().clone();
    let audio_streaming = state.audio_running.load(Ordering::SeqCst);
    let audio_device = state.audio_device_name.lock().unwrap().clone();

    ConnectionStatus {
        serial_connected,
        serial_port,
        audio_streaming,
        audio_device,
    }
}
