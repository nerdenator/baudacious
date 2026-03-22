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

/// Pure helper that reads `AppState` without requiring Tauri's `State` wrapper.
/// Extracted so it can be unit-tested directly.
fn connection_status_from_state(state: &AppState) -> ConnectionStatus {
    // Use into_inner() on poison so we report best-known state rather than a
    // misleading default (e.g. showing "disconnected" when the radio is actually
    // connected but the mutex was poisoned by a prior panic).
    let serial_connected = state
        .radio
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    let serial_port = state
        .serial_port_name
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let audio_streaming = state.audio_running.load(Ordering::SeqCst);
    let audio_device = state
        .audio_device_name
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    ConnectionStatus {
        serial_connected,
        serial_port,
        audio_streaming,
        audio_device,
    }
}

#[tauri::command]
pub fn get_connection_status(state: State<'_, AppState>) -> ConnectionStatus {
    connection_status_from_state(&state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mock_radio::MockRadio;
    use std::sync::atomic::Ordering;

    #[test]
    fn status_all_disconnected() {
        let state = AppState::new();
        let status = connection_status_from_state(&state);
        assert!(!status.serial_connected);
        assert!(status.serial_port.is_none());
        assert!(!status.audio_streaming);
        assert!(status.audio_device.is_none());
    }

    #[test]
    fn status_with_serial_connected() {
        let state = AppState::new();
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        *state.serial_port_name.lock().unwrap() = Some("/dev/ttyUSB0".to_string());

        let status = connection_status_from_state(&state);
        assert!(status.serial_connected);
        assert_eq!(status.serial_port.as_deref(), Some("/dev/ttyUSB0"));
        assert!(!status.audio_streaming);
    }

    #[test]
    fn status_with_audio_streaming() {
        let state = AppState::new();
        state.audio_running.store(true, Ordering::SeqCst);
        *state.audio_device_name.lock().unwrap() = Some("USB Audio CODEC".to_string());

        let status = connection_status_from_state(&state);
        assert!(!status.serial_connected);
        assert!(status.audio_streaming);
        assert_eq!(status.audio_device.as_deref(), Some("USB Audio CODEC"));
    }

    #[test]
    fn status_both_connected() {
        let state = AppState::new();
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        *state.serial_port_name.lock().unwrap() = Some("/dev/tty.usbserial".to_string());
        state.audio_running.store(true, Ordering::SeqCst);
        *state.audio_device_name.lock().unwrap() = Some("FT-991A USB".to_string());

        let status = connection_status_from_state(&state);
        assert!(status.serial_connected);
        assert_eq!(status.serial_port.as_deref(), Some("/dev/tty.usbserial"));
        assert!(status.audio_streaming);
        assert_eq!(status.audio_device.as_deref(), Some("FT-991A USB"));
    }
}
