//! Radio control commands — PTT, frequency, mode
//!
//! Each command locks the radio from AppState, checks it's connected,
//! calls the trait method, and maps errors to String for Tauri IPC.
//!
//! Serial I/O errors (Psk31Error::Serial) indicate physical disconnection.
//! with_radio() detects these, nulls out AppState.radio, and emits a
//! `serial-disconnected` event so the frontend can reset its UI automatically.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::domain::{Frequency, Psk31Error, Psk31Result, RadioStatus};
use crate::ports::RadioControl;
use crate::state::AppState;

/// Payload for the `serial-disconnected` event
#[derive(Clone, Serialize)]
struct SerialDisconnectedPayload {
    reason: String,
    port: String,
}

/// Lock the radio mutex, check it's connected, and run `f` on it.
///
/// On `Psk31Error::Serial` (physical I/O failure), automatically:
/// 1. Nulls out `AppState.radio` (marks as disconnected)
/// 2. Clears `AppState.serial_port_name`
/// 3. Emits `serial-disconnected` so the frontend resets its CAT UI
pub(crate) fn with_radio<T>(
    state: &State<AppState>,
    app: &AppHandle,
    f: impl FnOnce(&mut Box<dyn RadioControl>) -> Psk31Result<T>,
) -> Result<T, String> {
    let mut guard = state
        .radio
        .lock()
        .map_err(|_| "Radio state corrupted".to_string())?;
    let radio = guard.as_mut().ok_or("Radio not connected")?;

    match f(radio) {
        Ok(val) => Ok(val),
        Err(e @ Psk31Error::Serial(_)) => {
            // Serial I/O error — hardware is gone, auto-disconnect
            *guard = None;
            drop(guard); // Release radio mutex before acquiring port-name mutex
            let port = state.serial_port_name.lock().unwrap().take().unwrap_or_default();
            let _ = app.emit(
                "serial-disconnected",
                SerialDisconnectedPayload { reason: e.to_string(), port },
            );
            Err(e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn ptt_on(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    with_radio(&state, &app, |r| r.ptt_on())
}

#[tauri::command]
pub fn ptt_off(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    with_radio(&state, &app, |r| r.ptt_off())
}

#[tauri::command]
pub fn get_frequency(app: AppHandle, state: State<AppState>) -> Result<f64, String> {
    with_radio(&state, &app, |r| r.get_frequency().map(|f| f.as_hz()))
}

#[tauri::command]
pub fn set_frequency(app: AppHandle, state: State<AppState>, freq_hz: f64) -> Result<(), String> {
    with_radio(&state, &app, |r| r.set_frequency(Frequency::hz(freq_hz)))
}

#[tauri::command]
pub fn get_mode(app: AppHandle, state: State<AppState>) -> Result<String, String> {
    with_radio(&state, &app, |r| r.get_mode())
}

#[tauri::command]
pub fn set_mode(app: AppHandle, state: State<AppState>, mode: String) -> Result<(), String> {
    with_radio(&state, &app, |r| r.set_mode(&mode))
}

#[tauri::command]
pub fn get_signal_strength(app: AppHandle, state: State<AppState>) -> Result<f32, String> {
    with_radio(&state, &app, |r| r.get_signal_strength())
}

/// Returns frequency + mode in one IF; round-trip, used for periodic UI sync.
#[tauri::command]
pub fn get_radio_state(app: AppHandle, state: State<AppState>) -> Result<RadioStatus, String> {
    with_radio(&state, &app, |r| r.get_status())
}

#[tauri::command]
pub fn get_tx_power(app: AppHandle, state: State<AppState>) -> Result<u32, String> {
    with_radio(&state, &app, |r| r.get_tx_power()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Frequency, Psk31Result, RadioStatus};
    use crate::ports::RadioControl;

    /// Minimal mock radio whose tx_power field can be set for testing.
    struct MockRadio {
        tx_power: u32,
    }

    impl RadioControl for MockRadio {
        fn ptt_on(&mut self) -> Psk31Result<()> { Ok(()) }
        fn ptt_off(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_transmitting(&self) -> bool { false }
        fn get_frequency(&mut self) -> Psk31Result<Frequency> { Ok(Frequency::hz(14_070_000.0)) }
        fn set_frequency(&mut self, _freq: Frequency) -> Psk31Result<()> { Ok(()) }
        fn get_mode(&mut self) -> Psk31Result<String> { Ok("DATA-USB".to_string()) }
        fn set_mode(&mut self, _mode: &str) -> Psk31Result<()> { Ok(()) }
        fn get_tx_power(&mut self) -> Psk31Result<u32> { Ok(self.tx_power) }
        fn set_tx_power(&mut self, watts: u32) -> Psk31Result<()> {
            self.tx_power = watts;
            Ok(())
        }
        fn get_signal_strength(&mut self) -> Psk31Result<f32> { Ok(0.0) }
        fn get_status(&mut self) -> Psk31Result<RadioStatus> {
            Ok(RadioStatus {
                frequency_hz: 14_070_000,
                mode: "DATA-USB".to_string(),
                is_transmitting: false,
                rit_offset_hz: 0,
                rit_enabled: false,
                split: false,
            })
        }
    }

    /// Build an AppState with a mock radio pre-installed.
    fn make_state_with_radio(tx_power: u32) -> AppState {
        let state = AppState::new();
        let mock: Box<dyn RadioControl> = Box::new(MockRadio { tx_power });
        *state.radio.lock().unwrap() = Some(mock);
        state
    }

    #[test]
    fn get_tx_power_returns_value_from_radio() {
        let state = make_state_with_radio(10);
        // Verify the mock is set up correctly by calling through the trait object.
        // (The Tauri command itself can't be tested directly in unit tests without
        //  a real AppHandle, but this verifies the RadioControl delegate works.)
        let watts = state
            .radio
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .get_tx_power()
            .unwrap();
        assert_eq!(watts, 10);
    }

    #[test]
    fn get_tx_power_returns_configured_value() {
        let state = make_state_with_radio(50);
        let watts = state
            .radio
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .get_tx_power()
            .unwrap();
        assert_eq!(watts, 50);
    }
}
