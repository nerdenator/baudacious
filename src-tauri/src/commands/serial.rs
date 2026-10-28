//! Serial port commands — list, connect, disconnect

use crate::adapters::ft891::Ft891Radio;
use crate::adapters::ft991a::Ft991aRadio;
use crate::adapters::mock_radio::MockRadio;
use crate::adapters::serial_port::SerialPortFactory;
use crate::domain::{data_mode_for_frequency, RadioInfo, RadioType, SerialPortInfo};
use crate::ports::{RadioControl, SerialConnection, SerialFactory};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn list_serial_ports() -> Result<Vec<SerialPortInfo>, String> {
    if std::env::var("MOCK_RADIO").is_ok() {
        return Ok(vec![SerialPortInfo {
            name: "mock".to_string(),
            port_type: "Mock Radio".to_string(),
            device_hint: None,
        }]);
    }
    SerialPortFactory::list_ports().map_err(|e| e.to_string())
}

/// Pure connect logic: given a pre-constructed radio adapter, initialise it,
/// store it in `state`, and return the `RadioInfo`.  Extracted from the Tauri
/// command so it can be unit-tested without `State<'_, AppState>`.
fn connect_serial_inner(
    state: &AppState,
    display_port: String,
    baud_rate: u32,
    mut radio: Box<dyn RadioControl>,
) -> Result<RadioInfo, String> {
    // Auto-detect current state with separate FA; and MD0; queries.
    // Using FA; + MD0; avoids the firmware-variant ambiguity in IF; response parsing,
    // and FA; has no amateur-band restriction on read (unlike set_frequency).
    let frequency_hz = radio.get_frequency().map_err(|e| e.to_string())?.as_hz();
    let current_mode = radio.get_mode().map_err(|e| e.to_string())?;

    // Ensure the radio is in the correct DATA mode for this frequency.
    // BS; (used by set_frequency) recalls the band's stored mode, which may be a phone
    // mode (e.g. LSB) rather than DATA-LSB. Correct it here at connect time.
    let required_mode = data_mode_for_frequency(frequency_hz);
    let mode = if current_mode != required_mode {
        log::info!("connect: correcting mode {current_mode} → {required_mode} for {frequency_hz} Hz");
        if let Err(e) = radio.set_mode(required_mode) {
            log::warn!("connect: set_mode failed (continuing with current mode): {e}");
            current_mode
        } else {
            required_mode.to_string()
        }
    } else {
        current_mode
    };

    let info = RadioInfo {
        port: display_port.clone(),
        baud_rate,
        frequency_hz,
        mode,
        connected: true,
    };

    // Store radio in app state, then release the guard before locking serial_port_name
    // to avoid holding two MutexGuards simultaneously.
    {
        let mut radio_slot = state
            .radio
            .lock()
            .unwrap_or_else(|e| { log::warn!("connect: radio mutex was poisoned; recovering state"); e.into_inner() });
        *radio_slot = Some(radio);
    }
    // Use into_inner() on poison so the port name is always recorded, keeping
    // serial_port_name consistent with the radio slot we just set above.
    *state
        .serial_port_name
        .lock()
        .unwrap_or_else(|e| { log::warn!("connect: serial_port_name mutex was poisoned; recovering state"); e.into_inner() }) = Some(display_port);

    Ok(info)
}

/// Pure disconnect logic extracted from the Tauri command for testability.
fn disconnect_serial_inner(state: &AppState) -> Result<(), String> {
    {
        // Use into_inner() on poison so we still attempt to clear the radio slot
        // even if a previous thread panicked while holding this lock.
        let mut radio_slot = state.radio.lock().unwrap_or_else(|e| { log::warn!("disconnect: radio mutex was poisoned; recovering state"); e.into_inner() });
        // Drop will auto-release PTT if transmitting
        *radio_slot = None;
    } // radio guard released here before acquiring serial_port_name
    // Same best-effort clear: keep serial_port_name consistent with radio slot above.
    *state.serial_port_name.lock().unwrap_or_else(|e| { log::warn!("disconnect: serial_port_name mutex was poisoned; recovering state"); e.into_inner() }) = None;
    Ok(())
}

/// Pick the radio adapter for `radio_type` and wrap the open serial connection in it.
fn build_radio(radio_type: RadioType, connection: Box<dyn SerialConnection>) -> Box<dyn RadioControl> {
    match radio_type {
        RadioType::Ft991a => Box::new(Ft991aRadio::new(connection)),
        RadioType::Ft891 => Box::new(Ft891Radio::new(connection)),
    }
}

#[tauri::command]
pub fn connect_serial(
    state: State<AppState>,
    port: String,
    baud_rate: u32,
    radio_type: String,
) -> Result<RadioInfo, String> {
    // Reject unknown radios up front — never fall back to another radio's protocol.
    let radio_type: RadioType = radio_type.parse().map_err(|e: crate::domain::Psk31Error| e.to_string())?;
    let mock_mode = std::env::var("MOCK_RADIO").is_ok();

    let (radio, display_port): (Box<dyn RadioControl>, String) = if mock_mode {
        log::info!("[MOCK RADIO] MOCK_RADIO=1: skipping serial, using mock adapter");
        (Box::new(MockRadio::new()), "mock".to_string())
    } else {
        let connection = SerialPortFactory::open(&port, baud_rate).map_err(|e| e.to_string())?;
        (build_radio(radio_type, connection), port.clone())
    };

    connect_serial_inner(&state, display_port, baud_rate, radio)
}

#[tauri::command]
pub fn disconnect_serial(state: State<AppState>) -> Result<(), String> {
    disconnect_serial_inner(&state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mock_radio::MockRadio;
    use crate::domain::{Frequency, Psk31Result};

    /// Serial stub that records writes and answers every read with ";".
    struct StubSerial(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl SerialConnection for StubSerial {
        fn write(&mut self, data: &[u8]) -> Psk31Result<usize> {
            self.0.lock().unwrap().push(String::from_utf8_lossy(data).into());
            Ok(data.len())
        }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            buf[0] = b';';
            Ok(1)
        }
        fn close(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_connected(&self) -> bool { true }
    }

    fn stub() -> (Box<dyn SerialConnection>, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        (Box::new(StubSerial(log.clone())), log)
    }

    #[test]
    fn build_radio_ft891_enforces_ft891_limits() {
        let (conn, log) = stub();
        let mut radio = build_radio(RadioType::Ft891, conn);
        assert!(radio.set_tx_power(4).is_err(), "FT-891 minimum is 5 W");
        assert!(radio.set_frequency(Frequency::hz(144_200_000.0)).is_err(), "no 2m on FT-891");
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn build_radio_ft991a_enforces_ft991a_limits() {
        let (conn, log) = stub();
        let mut radio = build_radio(RadioType::Ft991a, conn);
        assert!(radio.set_frequency(Frequency::hz(144_200_000.0)).is_ok(), "2m is fine on FT-991A");
        assert_eq!(log.lock().unwrap()[0], "BS12;");
    }

    #[test]
    fn connect_corrects_wrong_mode_on_connect() {
        // MockRadio starts at 14.070 MHz DATA-USB. Override mode to USB (wrong for 20m)
        // so connect_serial_inner exercises the mode-correction branch.
        let state = AppState::new();
        let mut mock = MockRadio::new();
        mock.set_mode("USB").unwrap();
        let radio: Box<dyn RadioControl> = Box::new(mock);
        let info = connect_serial_inner(&state, "mock".to_string(), 38400, radio).unwrap();
        // connect_serial_inner should have corrected USB → DATA-USB
        assert_eq!(info.mode, "DATA-USB");
    }

    #[test]
    fn connect_with_mock_radio_returns_info() {
        let state = AppState::new();
        let radio: Box<dyn RadioControl> = Box::new(MockRadio::new());
        let info = connect_serial_inner(&state, "mock".to_string(), 38400, radio).unwrap();

        assert!(info.connected);
        assert_eq!(info.port, "mock");
        assert_eq!(info.baud_rate, 38400);
        // MockRadio defaults to 14_070_000 Hz DATA-USB
        assert_eq!(info.frequency_hz, 14_070_000.0);
        assert_eq!(info.mode, "DATA-USB");
    }

    #[test]
    fn connect_sets_state_correctly() {
        let state = AppState::new();
        let radio: Box<dyn RadioControl> = Box::new(MockRadio::new());
        connect_serial_inner(&state, "/dev/ttyUSB0".to_string(), 38400, radio).unwrap();

        assert!(state.radio.lock().unwrap().is_some());
        assert_eq!(
            state.serial_port_name.lock().unwrap().as_deref(),
            Some("/dev/ttyUSB0")
        );
    }

    #[test]
    fn disconnect_clears_state() {
        let state = AppState::new();
        // Pre-populate state as if a connect had succeeded
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        *state.serial_port_name.lock().unwrap() = Some("/dev/ttyUSB0".to_string());

        disconnect_serial_inner(&state).unwrap();

        assert!(state.radio.lock().unwrap().is_none());
        assert!(state.serial_port_name.lock().unwrap().is_none());
    }

    #[test]
    fn disconnect_when_already_disconnected_is_ok() {
        // Disconnecting when no radio is connected should succeed without error
        let state = AppState::new();
        assert!(disconnect_serial_inner(&state).is_ok());
        assert!(state.radio.lock().unwrap().is_none());
    }

    #[test]
    fn connect_then_disconnect_leaves_clean_state() {
        let state = AppState::new();
        let radio: Box<dyn RadioControl> = Box::new(MockRadio::new());
        connect_serial_inner(&state, "mock".to_string(), 38400, radio).unwrap();
        assert!(state.radio.lock().unwrap().is_some());

        disconnect_serial_inner(&state).unwrap();
        assert!(state.radio.lock().unwrap().is_none());
        assert!(state.serial_port_name.lock().unwrap().is_none());
    }
}
