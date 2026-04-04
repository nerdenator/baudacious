//! Application state

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use crate::domain::{ModemConfig, ModemStatus};
use crate::ports::RadioControl;

/// State of the TX pipeline.
///
/// Transitions:
/// - `Idle` → `Starting`: start_tx/start_tune claims the slot (under lock)
/// - `Starting` → `Running`: thread spawned and handle stored (under lock)
/// - `Running` → `Idle`: stop_tx/stop_tune takes handle (under lock), then joins
/// - `Starting` → `Idle`: stop_tx during slow work, or start_tx aborts early
/// - `Running` → `Idle`: run_tx_thread self-clears on normal completion
pub enum TxState {
    Idle,
    Starting,
    Running(JoinHandle<()>),
}

/// Shared application state managed by Tauri
pub struct AppState {
    pub config: Mutex<ModemConfig>,
    pub status: Mutex<ModemStatus>,
    pub radio: Mutex<Option<Box<dyn RadioControl>>>,
    /// Shared flag to signal the audio thread to stop
    pub audio_running: Arc<AtomicBool>,
    /// Handle to the audio processing thread (for clean shutdown)
    pub audio_thread: Mutex<Option<JoinHandle<()>>>,
    /// Shared flag to signal the TX thread to abort
    pub tx_abort: Arc<AtomicBool>,
    /// Monotonically increasing counter; incremented on every successful tx_claim.
    /// start_tx / start_tune capture their generation at claim time and compare it
    /// on abort checks and tx_activate to detect ABA races (stop → new start →
    /// old start continues).
    pub tx_generation: Arc<AtomicU64>,
    /// TX pipeline state: Idle / Starting / Running(handle)
    pub tx_state: Mutex<TxState>,
    /// Shared flag to enable/disable the RX decoder in the audio thread
    pub rx_running: Arc<AtomicBool>,
    /// Carrier frequency for RX decoder (updated by click-to-tune)
    pub rx_carrier_freq: Arc<Mutex<f64>>,
    /// Name of the currently active audio input device (None if not streaming).
    /// Wrapped in Arc so the audio thread can clear it on device loss.
    pub audio_device_name: Arc<Mutex<Option<String>>>,
    /// Name of the currently connected serial port (None if not connected)
    pub serial_port_name: Mutex<Option<String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(ModemConfig::default()),
            status: Mutex::new(ModemStatus::default()),
            radio: Mutex::new(None),
            audio_running: Arc::new(AtomicBool::new(false)),
            audio_thread: Mutex::new(None),
            tx_abort: Arc::new(AtomicBool::new(false)),
            tx_generation: Arc::new(AtomicU64::new(0)),
            tx_state: Mutex::new(TxState::Idle),
            rx_running: Arc::new(AtomicBool::new(false)),
            rx_carrier_freq: Arc::new(Mutex::new(1000.0)),
            audio_device_name: Arc::new(Mutex::new(None)),
            serial_port_name: Mutex::new(None),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn app_state_default_equals_new() {
        let a = AppState::new();
        let b = AppState::default();
        // Both start with audio_running = false
        assert!(!a.audio_running.load(Ordering::Relaxed));
        assert!(!b.audio_running.load(Ordering::Relaxed));
    }

    #[test]
    fn app_state_initial_flags_are_false() {
        let state = AppState::new();
        assert!(!state.audio_running.load(Ordering::Relaxed));
        assert!(!state.tx_abort.load(Ordering::Relaxed));
        assert!(!state.rx_running.load(Ordering::Relaxed));
    }

    #[test]
    fn tx_state_starts_idle() {
        let state = AppState::new();
        assert!(
            matches!(*state.tx_state.lock().unwrap(), TxState::Idle),
            "tx_state must start as Idle"
        );
    }

    #[test]
    fn app_state_rx_carrier_freq_default_is_1000() {
        let state = AppState::new();
        assert_eq!(*state.rx_carrier_freq.lock().unwrap(), 1000.0);
    }

    #[test]
    fn app_state_audio_device_name_starts_none() {
        let state = AppState::new();
        assert!(state.audio_device_name.lock().unwrap().is_none());
    }

    #[test]
    fn app_state_serial_port_name_starts_none() {
        let state = AppState::new();
        assert!(state.serial_port_name.lock().unwrap().is_none());
    }

    #[test]
    fn app_state_radio_starts_none() {
        let state = AppState::new();
        assert!(state.radio.lock().unwrap().is_none());
    }
}
