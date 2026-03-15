//! Serial port commands — list, connect, disconnect

use crate::adapters::ft991a::Ft991aRadio;
use crate::adapters::mock_radio::MockRadio;
use crate::adapters::serial_port::SerialPortFactory;
use crate::domain::{data_mode_for_frequency, RadioInfo, SerialPortInfo};
use crate::ports::{RadioControl, SerialFactory};
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

#[tauri::command]
pub fn connect_serial(
    state: State<AppState>,
    port: String,
    baud_rate: u32,
) -> Result<RadioInfo, String> {
    let mock_mode = std::env::var("MOCK_RADIO").is_ok();

    let (mut radio, display_port): (Box<dyn RadioControl>, String) = if mock_mode {
        log::info!("[MOCK RADIO] MOCK_RADIO=1: skipping serial, using mock adapter");
        (Box::new(MockRadio::new()), "mock".to_string())
    } else {
        // Open real serial connection and wrap in FT-991A adapter
        let connection = SerialPortFactory::open(&port, baud_rate).map_err(|e| e.to_string())?;
        (Box::new(Ft991aRadio::new(connection)), port.clone())
    };

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

    // Store radio and port name in app state
    let mut radio_slot = state.radio.lock().map_err(|_| "Radio state corrupted".to_string())?;
    *radio_slot = Some(radio);
    *state.serial_port_name.lock().map_err(|_| "Serial port state corrupted".to_string())? =
        Some(display_port);

    Ok(info)
}

#[tauri::command]
pub fn disconnect_serial(state: State<AppState>) -> Result<(), String> {
    let mut radio_slot = state.radio.lock().map_err(|_| "Radio state corrupted".to_string())?;
    // Drop will auto-release PTT if transmitting
    *radio_slot = None;
    *state.serial_port_name.lock().map_err(|_| "Serial port state corrupted".to_string())? =
        None;
    Ok(())
}
