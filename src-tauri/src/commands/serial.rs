//! Serial port commands — list, connect, disconnect

use crate::adapters::ft991a::Ft991aRadio;
use crate::adapters::mock_radio::MockRadio;
use crate::adapters::serial_port::SerialPortFactory;
use crate::domain::{RadioInfo, SerialPortInfo};
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

    // Auto-detect: one IF; query gives us freq + mode (+ RIT/split state) in a
    // single serial round-trip instead of the two separate FA; + MD0; calls.
    let status = radio.get_status().map_err(|e| e.to_string())?;
    let frequency_hz = status.frequency_hz as f64;
    let mode = status.mode;

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
