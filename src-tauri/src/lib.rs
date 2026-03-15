//! PSK-31 Desktop Client
//!
//! A cross-platform application for sending and receiving PSK-31 ham radio messages.
//!
//! ## Architecture (Hexagonal / Ports & Adapters)
//!
//! - `domain/` - Pure domain types, no I/O dependencies
//! - `ports/` - Trait definitions (interfaces) for external dependencies
//! - `dsp/` - Signal processing (pure functions, no I/O)
//! - `modem/` - PSK-31 protocol logic (varicode, encoder, decoder)
//! - `adapters/` - Implementations of ports (cpal audio, serialport, FT-991A)
//! - `commands/` - Tauri command handlers (driving adapters)
//! - `state/` - Application state management

// Core domain (pure, no I/O)
pub mod domain;
pub mod dsp;
pub mod modem;
pub mod ports;

// CAT command layer (encode/decode/session)
pub mod cat;

// Adapters (external I/O)
pub mod adapters;

// Tauri integration
pub mod commands;
pub mod menu;
pub mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();
    log::info!("Baudacious starting");
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .setup(|app| {
            menu::setup_menu(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // App commands
            commands::app::exit_app,
            // Audio commands
            commands::audio::list_audio_devices,
            commands::audio::start_audio_stream,
            commands::audio::stop_audio_stream,
            // RX commands
            commands::audio::start_rx,
            commands::audio::stop_rx,
            commands::audio::set_carrier_frequency,
            // Serial commands
            commands::serial::list_serial_ports,
            commands::serial::connect_serial,
            commands::serial::disconnect_serial,
            // Radio commands
            commands::radio::ptt_on,
            commands::radio::ptt_off,
            commands::radio::get_frequency,
            commands::radio::set_frequency,
            commands::radio::get_mode,
            commands::radio::set_mode,
            commands::radio::get_signal_strength,
            commands::radio::get_radio_state,
            commands::radio::get_tx_power,
            // TX commands
            commands::tx::start_tx,
            commands::tx::stop_tx,
            // Configuration commands
            commands::config::save_configuration,
            commands::config::load_configuration,
            commands::config::list_configurations,
            commands::config::delete_configuration,
            commands::config::set_tx_power_config,
            // Status command
            commands::status::get_connection_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
