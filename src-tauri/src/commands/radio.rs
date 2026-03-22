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

/// Pure inner logic for `with_radio` — operates on raw Option slots instead of
/// AppState so it can be tested without an AppHandle.
///
/// Returns:
/// - `Ok(T)` on success
/// - `Err((msg, true))` when a `Psk31Error::Serial` occurred (radio nulled, port cleared)
/// - `Err((msg, false))` for any other error (radio left intact)
pub(crate) fn with_radio_inner<T>(
    radio: &mut Option<Box<dyn RadioControl>>,
    serial_port_name: &mut Option<String>,
    f: impl FnOnce(&mut Box<dyn RadioControl>) -> Psk31Result<T>,
) -> Result<T, (String, bool)> {
    let r = radio.as_mut().ok_or(("Radio not connected".to_string(), false))?;

    match f(r) {
        Ok(val) => Ok(val),
        Err(e @ Psk31Error::Serial(_)) => {
            *radio = None;
            *serial_port_name = None;
            Err((e.to_string(), true))
        }
        Err(e) => Err((e.to_string(), false)),
    }
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

    let mut port_name_opt: Option<String>;
    {
        // We need to pass a mutable ref to serial_port_name into with_radio_inner,
        // but we can't hold two MutexGuards at once (deadlock risk). We snapshot
        // the port name into a local, let inner logic clear it, then write back.
        port_name_opt = state.serial_port_name.lock().unwrap().clone();
    }

    // Snapshot the port name *before* with_radio_inner can null it out.
    // with_radio_inner sets *serial_port_name = None on serial errors,
    // so we must capture the original value here for the event payload.
    let port_for_event = port_name_opt.clone().unwrap_or_default();

    let result = with_radio_inner(&mut guard, &mut port_name_opt, f);

    // Drop the radio guard before any further locking or I/O.
    // The original code had an explicit drop(guard) here; preserving that avoids
    // holding the radio mutex across serial_port_name.lock() and app.emit().
    drop(guard);

    match result {
        Ok(val) => Ok(val),
        Err((msg, was_serial)) => {
            if was_serial {
                // Flush the nulled port name back to AppState
                *state
                    .serial_port_name
                    .lock()
                    .map_err(|_| "Serial port state corrupted".to_string())? = None;
                let port = port_for_event;
                let _ = app.emit(
                    "serial-disconnected",
                    SerialDisconnectedPayload { reason: msg.clone(), port },
                );
            }
            Err(msg)
        }
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

    // ---------------------------------------------------------------------------
    // Configurable mock radio for with_radio_inner tests
    // ---------------------------------------------------------------------------

    enum MockBehavior {
        /// Always return Ok(())
        Ok,
        /// Return Psk31Error::Serial
        SerialError,
        /// Return Psk31Error::Cat (non-serial error)
        CatError,
    }

    struct ConfigurableMock {
        tx_power: u32,
        behavior: MockBehavior,
    }

    impl ConfigurableMock {
        fn ok(tx_power: u32) -> Self {
            Self { tx_power, behavior: MockBehavior::Ok }
        }
        fn serial_err() -> Self {
            Self { tx_power: 0, behavior: MockBehavior::SerialError }
        }
        fn cat_err() -> Self {
            Self { tx_power: 0, behavior: MockBehavior::CatError }
        }
    }

    impl RadioControl for ConfigurableMock {
        fn ptt_on(&mut self) -> Psk31Result<()> {
            match self.behavior {
                MockBehavior::Ok => Ok(()),
                MockBehavior::SerialError => Err(Psk31Error::Serial("disconnected".into())),
                MockBehavior::CatError => Err(Psk31Error::Cat("bad command".into())),
            }
        }
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

    // ---------------------------------------------------------------------------
    // Helper: build a boxed radio slot
    // ---------------------------------------------------------------------------

    fn radio_slot(mock: ConfigurableMock) -> Option<Box<dyn RadioControl>> {
        Some(Box::new(mock))
    }

    // ---------------------------------------------------------------------------
    // with_radio_inner tests
    // ---------------------------------------------------------------------------

    #[test]
    fn with_radio_inner_succeeds_when_radio_present() {
        let mut slot = radio_slot(ConfigurableMock::ok(42));
        let mut port = Some("COM1".to_string());
        let result = with_radio_inner(&mut slot, &mut port, |r| r.ptt_on());
        assert!(result.is_ok());
        // Radio and port name should be untouched on success
        assert!(slot.is_some());
        assert_eq!(port.as_deref(), Some("COM1"));
    }

    #[test]
    fn with_radio_inner_errors_when_no_radio() {
        let mut slot: Option<Box<dyn RadioControl>> = None;
        let mut port = Some("COM1".to_string());
        let result = with_radio_inner(&mut slot, &mut port, |r| r.ptt_on());
        let Err((msg, was_serial)) = result else { panic!("expected Err") };
        assert_eq!(msg, "Radio not connected");
        assert!(!was_serial, "absent radio should not flag as serial error");
    }

    #[test]
    fn with_radio_inner_nulls_radio_on_serial_error() {
        let mut slot = radio_slot(ConfigurableMock::serial_err());
        let mut port = Some("COM1".to_string());
        let result = with_radio_inner(&mut slot, &mut port, |r| r.ptt_on());
        assert!(result.is_err());
        assert!(slot.is_none(), "radio should be nulled after serial error");
    }

    #[test]
    fn with_radio_inner_clears_port_name_on_serial_error() {
        let mut slot = radio_slot(ConfigurableMock::serial_err());
        let mut port = Some("COM1".to_string());
        let _ = with_radio_inner(&mut slot, &mut port, |r| r.ptt_on());
        assert!(port.is_none(), "port name should be cleared after serial error");
    }

    #[test]
    fn with_radio_inner_serial_error_sets_was_serial_flag() {
        let mut slot = radio_slot(ConfigurableMock::serial_err());
        let mut port = Some("COM1".to_string());
        let Err((_msg, was_serial)) = with_radio_inner(&mut slot, &mut port, |r| r.ptt_on()) else {
            panic!("expected Err")
        };
        assert!(was_serial, "Psk31Error::Serial should set was_serial = true");
    }

    #[test]
    fn with_radio_inner_preserves_radio_on_other_error() {
        let mut slot = radio_slot(ConfigurableMock::cat_err());
        let mut port = Some("COM1".to_string());
        let result = with_radio_inner(&mut slot, &mut port, |r| r.ptt_on());
        let Err((_msg, was_serial)) = result else { panic!("expected Err") };
        assert!(!was_serial, "CAT error should not set was_serial flag");
        assert!(slot.is_some(), "radio slot must survive a non-serial error");
        assert_eq!(port.as_deref(), Some("COM1"), "port name must survive a non-serial error");
    }

    // ---------------------------------------------------------------------------
    // Legacy tests that verify RadioControl trait dispatch (kept for regression)
    // ---------------------------------------------------------------------------

    /// Build an AppState with a mock radio pre-installed.
    fn make_state_with_radio(tx_power: u32) -> AppState {
        let state = AppState::new();
        let mock: Box<dyn RadioControl> = Box::new(ConfigurableMock::ok(tx_power));
        *state.radio.lock().unwrap() = Some(mock);
        state
    }

    #[test]
    fn get_tx_power_returns_value_from_radio() {
        let state = make_state_with_radio(10);
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
