//! TX commands — start/stop PSK-31 transmission
//!
//! The TX pipeline:
//! 1. Encode text to BPSK-31 samples (upfront, not streaming)
//! 2. Spawn a TX thread that:
//!    - Activates PTT (if radio connected)
//!    - Waits 50ms for PTT settle
//!    - Plays the samples via CpalAudioOutput
//!    - Emits progress events to the frontend
//!    - Deactivates PTT on both abort and complete paths
//!    - Emits a `tx-status: complete` or `tx-status: aborted` event
//! 3. stop_tx signals abort and calls PTT OFF as a belt-and-suspenders safety net
//!
//! ## Concurrency model
//!
//! `AppState.tx_state` (a `Mutex<TxState>`) guards the TX pipeline lifecycle:
//!
//! - `Idle`            — no TX, safe to start
//! - `Starting`        — claimed by a start call; slow work (encode, serial I/O) in progress
//! - `Running(handle)` — thread live, handle stored
//!
//! start_tx/start_tune hold the lock for only two brief critical sections:
//!   1. Claim: `Idle → Starting` + reset tx_abort (atomic, no I/O)
//!   2. Activate: `Starting → Running(handle)` after spawn
//!
//! stop_tx/stop_tune can therefore acquire the lock immediately (even during slow
//! work), set tx_abort=true, and transition state back to Idle without waiting for
//! serial round-trips to complete.

use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::adapters::cpal_audio::CpalAudioOutput;
use crate::commands::radio::with_radio;
use crate::domain::data_mode_for_frequency;
use crate::modem::encoder::Psk31Encoder;
use crate::ports::{AudioOutput, RadioControl};
use crate::state::{AppState, TxState};

/// Query the radio's current frequency and mode; if the mode is not the correct
/// DATA variant for that frequency, correct it.
///
/// Non-fatal: any error is logged as a warning and TX proceeds regardless.
fn ensure_data_mode(radio: &mut dyn RadioControl) {
    let hz = match radio.get_frequency() {
        Ok(f) => f.as_hz(),
        Err(e) => {
            log::warn!("Mode guard: could not read frequency: {e}");
            return;
        }
    };

    let target = data_mode_for_frequency(hz);

    let current = match radio.get_mode() {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Mode guard: could not read mode: {e}");
            return;
        }
    };

    if current != target {
        log::info!(
            "Mode guard: correcting {current} → {target} for {:.3} MHz",
            hz / 1e6
        );
        if let Err(e) = radio.set_mode(target) {
            log::warn!("Mode guard: set_mode({target}) failed: {e}");
        }
    }
}

/// Pure validation for `start_tx` / `start_tune` — no I/O, fully unit-testable.
///
/// Returns `Err` if the state machine is not `Idle` (i.e., TX is already in
/// progress or being set up).
pub(crate) fn validate_tx_state(tx: &TxState) -> Result<(), String> {
    match tx {
        TxState::Idle => Ok(()),
        _ => Err("Already transmitting".into()),
    }
}

/// Payload for `tx-status` events sent to the frontend
#[derive(Clone, Serialize)]
struct TxStatusPayload {
    status: String,
    progress: f32,
}

#[tauri::command]
pub fn start_tx(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
    device_id: String,
) -> Result<(), String> {
    // ── Critical section 1: claim + reset abort ───────────────────────────────
    // Both operations happen under the lock so a concurrent stop_tx that sets
    // tx_abort=true cannot have its signal erased by our reset.
    {
        let mut tx = state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())?;
        validate_tx_state(&tx)?;
        state.tx_abort.store(false, Ordering::SeqCst);
        *tx = TxState::Starting;
    }

    // ── Slow work (lock-free) ─────────────────────────────────────────────────
    // stop_tx can now acquire tx_state immediately and set tx_abort=true.

    let carrier_freq = state.config.lock().unwrap().carrier_freq;
    let sample_rate = state.config.lock().unwrap().sample_rate;
    let target_watts = state.config.lock().unwrap().tx_power_watts;

    let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
    let samples = encoder.encode(&text);

    if samples.is_empty() {
        *state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())? = TxState::Idle;
        return Err("Nothing to transmit".into());
    }

    let _ = with_radio(&state, &app, |radio| {
        ensure_data_mode(radio.as_mut());
        if let Err(e) = radio.set_tx_power(target_watts) {
            log::warn!("TX power set failed (continuing): {e}");
        }
        Ok(())
    });

    // ── Abort check ───────────────────────────────────────────────────────────
    // If stop_tx fired during slow work it set tx_abort=true. Cancel cleanly.
    if state.tx_abort.load(Ordering::SeqCst) {
        *state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())? = TxState::Idle;
        let _ = app.emit(
            "tx-status",
            TxStatusPayload { status: "aborted".into(), progress: 0.0 },
        );
        return Err("Cancelled before transmitting".into());
    }

    // ── Spawn ─────────────────────────────────────────────────────────────────
    let abort = state.tx_abort.clone();
    let play_pos = Arc::new(AtomicUsize::new(0));
    let total_samples = samples.len();

    let handle = {
        let abort = abort.clone();
        let play_pos = play_pos.clone();
        thread::spawn(move || {
            run_tx_thread(app, abort, play_pos, samples, device_id, total_samples);
        })
    };

    // ── Critical section 2: activate ─────────────────────────────────────────
    // If stop_tx raced between the abort check and here, state is already Idle.
    // The spawned thread will see tx_abort=true and exit on its own.
    {
        let mut tx = state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())?;
        if matches!(*tx, TxState::Starting) {
            *tx = TxState::Running(handle);
        }
        // else: stop_tx already reset to Idle; thread self-exits via abort flag.
    }

    Ok(())
}

fn stop_tx_inner(state: &AppState) -> Result<(), String> {
    // Lock first, set abort, take handle — all under the same guard so that
    // start_tx (which resets tx_abort=false inside the lock) cannot race with us.
    let handle = {
        let mut tx = state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())?;
        state.tx_abort.store(true, Ordering::SeqCst);
        match std::mem::replace(&mut *tx, TxState::Idle) {
            TxState::Running(h) => Some(h),
            _ => None, // Idle or Starting — abort flag is sufficient
        }
    }; // lock released before join to avoid deadlock with run_tx_thread's try_lock

    if let Some(handle) = handle {
        handle.join().map_err(|_| "TX thread panicked".to_string())?;
    }

    match state.radio.lock() {
        Ok(mut guard) => {
            if let Some(radio) = guard.as_mut() {
                if let Err(e) = radio.ptt_off() {
                    log::warn!("PTT OFF failed: {e}");
                }
            }
        }
        Err(_) => log::warn!("stop_tx: radio mutex poisoned, skipping PTT off"),
    }

    Ok(())
}

#[tauri::command]
pub fn stop_tx(state: tauri::State<'_, AppState>) -> Result<(), String> {
    stop_tx_inner(&state)
}

#[tauri::command]
pub fn start_tune(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    // ── Critical section 1: claim + reset abort ───────────────────────────────
    {
        let mut tx = state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())?;
        validate_tx_state(&tx)?;
        state.tx_abort.store(false, Ordering::SeqCst);
        *tx = TxState::Starting;
    }

    // ── Slow work (lock-free) ─────────────────────────────────────────────────
    let carrier_freq = state.config.lock().unwrap().carrier_freq;
    let sample_rate = state.config.lock().unwrap().sample_rate;

    // Tune always uses 10W regardless of the configured TX power setting
    let _ = with_radio(&state, &app, |radio| {
        ensure_data_mode(radio.as_mut());
        if let Err(e) = radio.set_tx_power(10) {
            log::warn!("TX power set failed (continuing): {e}");
        }
        Ok(())
    });

    // ── Abort check ───────────────────────────────────────────────────────────
    if state.tx_abort.load(Ordering::SeqCst) {
        *state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())? = TxState::Idle;
        let _ = app.emit(
            "tx-status",
            TxStatusPayload { status: "aborted".into(), progress: 0.0 },
        );
        return Err("Cancelled before transmitting".into());
    }

    // ── Spawn ─────────────────────────────────────────────────────────────────
    let abort = state.tx_abort.clone();

    let handle = thread::spawn(move || {
        run_tune_thread(app, abort, device_id, carrier_freq, f64::from(sample_rate));
    });

    // ── Critical section 2: activate ─────────────────────────────────────────
    {
        let mut tx = state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())?;
        if matches!(*tx, TxState::Starting) {
            *tx = TxState::Running(handle);
        }
    }

    Ok(())
}

fn stop_tune_inner(state: &AppState) -> Result<(), String> {
    let handle = {
        let mut tx = state
            .tx_state
            .lock()
            .map_err(|_| "TX state corrupted".to_string())?;
        state.tx_abort.store(true, Ordering::SeqCst);
        match std::mem::replace(&mut *tx, TxState::Idle) {
            TxState::Running(h) => Some(h),
            _ => None,
        }
    }; // lock released before join

    if let Some(handle) = handle {
        handle.join().map_err(|_| "Tune thread panicked".to_string())?;
    }

    let configured_watts = state.config.lock().unwrap().tx_power_watts;
    match state.radio.lock() {
        Ok(mut guard) => {
            if let Some(radio) = guard.as_mut() {
                if let Err(e) = radio.ptt_off() {
                    log::warn!("PTT OFF failed: {e}");
                }
                if let Err(e) = radio.set_tx_power(configured_watts) {
                    log::warn!("TX power restore failed: {e}");
                }
            }
        }
        Err(_) => log::warn!("stop_tune: radio mutex poisoned, skipping PTT off and power restore"),
    }

    Ok(())
}

#[tauri::command]
pub fn stop_tune(state: tauri::State<'_, AppState>) -> Result<(), String> {
    stop_tune_inner(&state)
}

/// Tune thread: transmits a continuous sine wave at the carrier frequency until aborted.
fn run_tune_thread(
    app: AppHandle,
    abort: Arc<std::sync::atomic::AtomicBool>,
    device_id: String,
    carrier_freq: f64,
    sample_rate: f64,
) {
    let radio_state = app.state::<AppState>();

    // PTT ON
    if let Ok(mut guard) = radio_state.radio.lock() {
        if let Some(radio) = guard.as_mut() {
            if let Err(e) = radio.ptt_on() {
                log::warn!("PTT ON failed (continuing without PTT): {e}");
            }
        }
    }

    thread::sleep(Duration::from_millis(50));

    let _ = app.emit(
        "tx-status",
        TxStatusPayload {
            status: "tuning".into(),
            progress: 0.0,
        },
    );

    let phase_inc = 2.0 * std::f64::consts::PI * carrier_freq / sample_rate;
    let abort_for_cb = abort.clone();

    let mut audio_output = CpalAudioOutput::new();
    let mut phase: f64 = 0.0;

    let start_result = audio_output.start(
        &device_id,
        Box::new(move |output_buf: &mut [f32]| {
            if abort_for_cb.load(Ordering::SeqCst) {
                for s in output_buf.iter_mut() {
                    *s = 0.0;
                }
                return;
            }
            for s in output_buf.iter_mut() {
                *s = phase.sin() as f32;
                phase += phase_inc;
                if phase > 2.0 * std::f64::consts::PI {
                    phase -= 2.0 * std::f64::consts::PI;
                }
            }
        }),
    );

    if let Err(e) = start_result {
        log::error!("Failed to start audio output for tune: {e}");
        if let Ok(mut guard) = radio_state.radio.lock() {
            if let Some(radio) = guard.as_mut() {
                let _ = radio.ptt_off();
            }
        }
        return;
    }

    loop {
        if abort.load(Ordering::SeqCst) {
            let _ = audio_output.stop();
            if let Ok(mut guard) = radio_state.radio.lock() {
                if let Some(radio) = guard.as_mut() {
                    if let Err(e) = radio.ptt_off() {
                        log::warn!("PTT OFF failed: {e}");
                    }
                }
            }
            let _ = app.emit(
                "tx-status",
                TxStatusPayload {
                    status: "aborted".into(),
                    progress: 0.0,
                },
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// TX thread: plays encoded samples through the audio output device.
fn run_tx_thread(
    app: AppHandle,
    abort: Arc<std::sync::atomic::AtomicBool>,
    play_pos: Arc<AtomicUsize>,
    samples: Vec<f32>,
    device_id: String,
    total_samples: usize,
) {
    // Activate PTT at the top of the thread (before the settle delay)
    let radio_state = app.state::<AppState>();
    if let Ok(mut guard) = radio_state.radio.lock() {
        if let Some(radio) = guard.as_mut() {
            if let Err(e) = radio.ptt_on() {
                log::warn!("PTT ON failed (continuing without PTT): {e}");
            }
        }
    }

    // Brief delay after PTT to let the radio switch to TX
    thread::sleep(Duration::from_millis(50));

    let _ = app.emit(
        "tx-status",
        TxStatusPayload {
            status: "transmitting".into(),
            progress: 0.0,
        },
    );

    // Set up audio output with a callback that pulls from our sample buffer
    let mut audio_output = CpalAudioOutput::new();
    let samples_arc = Arc::new(samples);
    let samples_for_callback = samples_arc.clone();
    let pos_for_callback = play_pos.clone();
    let done_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_for_callback = done_flag.clone();

    let start_result = audio_output.start(
        &device_id,
        Box::new(move |output_buf: &mut [f32]| {
            let current_pos = pos_for_callback.load(Ordering::Relaxed);
            let remaining = total_samples.saturating_sub(current_pos);

            if remaining == 0 {
                for sample in output_buf.iter_mut() {
                    *sample = 0.0;
                }
                done_for_callback.store(true, Ordering::SeqCst);
                return;
            }

            let copy_len = output_buf.len().min(remaining);
            output_buf[..copy_len]
                .copy_from_slice(&samples_for_callback[current_pos..current_pos + copy_len]);

            for sample in output_buf[copy_len..].iter_mut() {
                *sample = 0.0;
            }

            pos_for_callback.store(current_pos + copy_len, Ordering::Relaxed);
        }),
    );

    if let Err(e) = start_result {
        log::error!("Failed to start audio output: {e}");
        let _ = app.emit(
            "tx-status",
            TxStatusPayload {
                status: format!("error: {e}"),
                progress: 0.0,
            },
        );
        return;
    }

    // Wait for playback to finish or abort
    loop {
        if abort.load(Ordering::SeqCst) {
            let _ = audio_output.stop();
            let _ = app.emit(
                "tx-status",
                TxStatusPayload {
                    status: "aborted".into(),
                    progress: play_pos.load(Ordering::Relaxed) as f32 / total_samples as f32,
                },
            );

            if let Ok(mut guard) = radio_state.radio.lock() {
                if let Some(radio) = guard.as_mut() {
                    if let Err(e) = radio.ptt_off() {
                        log::warn!("PTT OFF failed: {e}");
                    }
                }
            }
            return;
        }

        if done_flag.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(30));
            let _ = audio_output.stop();

            // Emit complete BEFORE PTT OFF — UI resets with zero IPC latency.
            let _ = app.emit(
                "tx-status",
                TxStatusPayload {
                    status: "complete".into(),
                    progress: 1.0,
                },
            );

            // Self-clear our handle from AppState so start_tx works immediately.
            // Use try_lock to avoid deadlock if stop_tx holds the lock concurrently.
            if let Ok(mut tx) = radio_state.tx_state.try_lock() {
                if matches!(*tx, TxState::Running(_)) {
                    *tx = TxState::Idle;
                }
            }

            if let Ok(mut guard) = radio_state.radio.lock() {
                if let Some(radio) = guard.as_mut() {
                    if let Err(e) = radio.ptt_off() {
                        log::warn!("PTT OFF failed: {e}");
                    }
                }
            }

            return;
        }

        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mock_radio::MockRadio;
    use crate::domain::{Frequency, Psk31Error, Psk31Result, RadioStatus};
    use crate::ports::RadioControl;

    // -----------------------------------------------------------------------
    // validate_tx_state
    // -----------------------------------------------------------------------

    #[test]
    fn validate_tx_state_ok_when_idle() {
        assert!(validate_tx_state(&TxState::Idle).is_ok());
    }

    #[test]
    fn validate_tx_state_err_when_starting() {
        let err = validate_tx_state(&TxState::Starting).unwrap_err();
        assert_eq!(err, "Already transmitting");
    }

    #[test]
    fn validate_tx_state_err_when_running() {
        let handle = thread::spawn(|| {});
        let err = validate_tx_state(&TxState::Running(handle)).unwrap_err();
        assert_eq!(err, "Already transmitting");
    }

    // -----------------------------------------------------------------------
    // ensure_data_mode
    // -----------------------------------------------------------------------

    struct ModeMock {
        freq_hz: f64,
        current_mode: String,
        freq_err: Option<String>,
        mode_err: Option<String>,
        set_mode_err: Option<String>,
        set_mode_called_with: Option<String>,
    }

    impl ModeMock {
        fn correct_mode() -> Self {
            Self {
                freq_hz: 14_070_000.0,
                current_mode: "DATA-USB".to_string(),
                freq_err: None,
                mode_err: None,
                set_mode_err: None,
                set_mode_called_with: None,
            }
        }
        fn wrong_mode() -> Self {
            Self {
                freq_hz: 14_070_000.0,
                current_mode: "USB".to_string(),
                freq_err: None,
                mode_err: None,
                set_mode_err: None,
                set_mode_called_with: None,
            }
        }
        fn freq_error() -> Self {
            Self {
                freq_hz: 0.0,
                current_mode: "USB".to_string(),
                freq_err: Some("read failed".to_string()),
                mode_err: None,
                set_mode_err: None,
                set_mode_called_with: None,
            }
        }
        fn mode_read_error() -> Self {
            Self {
                freq_hz: 14_070_000.0,
                current_mode: "USB".to_string(),
                freq_err: None,
                mode_err: Some("mode read failed".to_string()),
                set_mode_err: None,
                set_mode_called_with: None,
            }
        }
        fn set_mode_error() -> Self {
            Self {
                freq_hz: 14_070_000.0,
                current_mode: "USB".to_string(),
                freq_err: None,
                mode_err: None,
                set_mode_err: Some("set failed".to_string()),
                set_mode_called_with: None,
            }
        }
    }

    impl RadioControl for ModeMock {
        fn ptt_on(&mut self) -> Psk31Result<()> { Ok(()) }
        fn ptt_off(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_transmitting(&self) -> bool { false }
        fn get_frequency(&mut self) -> Psk31Result<Frequency> {
            if let Some(ref e) = self.freq_err {
                return Err(Psk31Error::Serial(e.clone()));
            }
            Ok(Frequency::hz(self.freq_hz))
        }
        fn set_frequency(&mut self, _freq: Frequency) -> Psk31Result<()> { Ok(()) }
        fn get_mode(&mut self) -> Psk31Result<String> {
            if let Some(ref e) = self.mode_err {
                return Err(Psk31Error::Cat(e.clone()));
            }
            Ok(self.current_mode.clone())
        }
        fn set_mode(&mut self, mode: &str) -> Psk31Result<()> {
            self.set_mode_called_with = Some(mode.to_string());
            if let Some(ref e) = self.set_mode_err {
                return Err(Psk31Error::Cat(e.clone()));
            }
            self.current_mode = mode.to_string();
            Ok(())
        }
        fn get_tx_power(&mut self) -> Psk31Result<u32> { Ok(25) }
        fn set_tx_power(&mut self, _watts: u32) -> Psk31Result<()> { Ok(()) }
        fn get_signal_strength(&mut self) -> Psk31Result<f32> { Ok(0.0) }
        fn get_status(&mut self) -> Psk31Result<RadioStatus> {
            Ok(RadioStatus {
                frequency_hz: self.freq_hz as u64,
                mode: self.current_mode.clone(),
                is_transmitting: false,
                rit_offset_hz: 0,
                rit_enabled: false,
                split: false,
            })
        }
    }

    #[test]
    fn ensure_data_mode_noop_when_already_correct() {
        let mut mock = ModeMock::correct_mode();
        ensure_data_mode(&mut mock);
        assert!(mock.set_mode_called_with.is_none());
    }

    #[test]
    fn ensure_data_mode_corrects_wrong_mode() {
        let mut mock = ModeMock::wrong_mode();
        ensure_data_mode(&mut mock);
        assert_eq!(mock.set_mode_called_with.as_deref(), Some("DATA-USB"));
    }

    #[test]
    fn ensure_data_mode_skips_on_freq_read_error() {
        let mut mock = ModeMock::freq_error();
        ensure_data_mode(&mut mock);
        assert!(mock.set_mode_called_with.is_none());
    }

    #[test]
    fn ensure_data_mode_skips_on_mode_read_error() {
        let mut mock = ModeMock::mode_read_error();
        ensure_data_mode(&mut mock);
        assert!(mock.set_mode_called_with.is_none());
    }

    #[test]
    fn ensure_data_mode_tolerates_set_mode_failure() {
        let mut mock = ModeMock::set_mode_error();
        ensure_data_mode(&mut mock);
        assert_eq!(mock.set_mode_called_with.as_deref(), Some("DATA-USB"));
    }

    #[test]
    fn ensure_data_mode_lsb_below_10mhz() {
        let mut mock = ModeMock {
            freq_hz: 7_074_000.0,
            current_mode: "USB".to_string(),
            freq_err: None,
            mode_err: None,
            set_mode_err: None,
            set_mode_called_with: None,
        };
        ensure_data_mode(&mut mock);
        assert_eq!(mock.set_mode_called_with.as_deref(), Some("DATA-LSB"));
    }

    // -----------------------------------------------------------------------
    // stop_tx_inner
    // -----------------------------------------------------------------------

    #[test]
    fn stop_tx_inner_when_idle_ok() {
        let state = AppState::new();
        stop_tx_inner(&state).unwrap();
        assert!(state.tx_abort.load(Ordering::SeqCst), "abort flag must be set");
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }

    #[test]
    fn stop_tx_inner_when_starting_sets_abort_and_idles() {
        let state = AppState::new();
        *state.tx_state.lock().unwrap() = TxState::Starting;
        stop_tx_inner(&state).unwrap();
        assert!(state.tx_abort.load(Ordering::SeqCst));
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }

    #[test]
    fn stop_tx_inner_calls_ptt_off_when_radio_present() {
        let state = AppState::new();
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        stop_tx_inner(&state).unwrap();
    }

    #[test]
    fn stop_tx_inner_joins_thread() {
        let state = AppState::new();
        let handle = thread::spawn(|| {});
        *state.tx_state.lock().unwrap() = TxState::Running(handle);
        stop_tx_inner(&state).unwrap();
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }

    // -----------------------------------------------------------------------
    // stop_tune_inner
    // -----------------------------------------------------------------------

    #[test]
    fn stop_tune_inner_when_idle_ok() {
        let state = AppState::new();
        stop_tune_inner(&state).unwrap();
        assert!(state.tx_abort.load(Ordering::SeqCst));
    }

    #[test]
    fn stop_tune_inner_when_starting_sets_abort_and_idles() {
        let state = AppState::new();
        *state.tx_state.lock().unwrap() = TxState::Starting;
        stop_tune_inner(&state).unwrap();
        assert!(state.tx_abort.load(Ordering::SeqCst));
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }

    #[test]
    fn stop_tune_inner_calls_ptt_off_and_restores_tx_power() {
        let state = AppState::new();
        state.config.lock().unwrap().tx_power_watts = 50;
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        stop_tune_inner(&state).unwrap();
    }

    #[test]
    fn stop_tune_inner_joins_thread() {
        let state = AppState::new();
        let handle = thread::spawn(|| {});
        *state.tx_state.lock().unwrap() = TxState::Running(handle);
        stop_tune_inner(&state).unwrap();
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }
}
