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
use crate::state::AppState;

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
/// Arguments:
/// - `is_transmitting`: `true` when a tx/tune thread is already running
///
/// Returns `Err` with a human-readable message when TX should be blocked.
pub(crate) fn validate_tx_start(is_transmitting: bool) -> Result<(), String> {
    if is_transmitting {
        return Err("Already transmitting".into());
    }
    Ok(())
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
    validate_tx_start(state.tx_thread.lock().unwrap().is_some())?;

    // Read carrier frequency from config
    let carrier_freq = state.config.lock().unwrap().carrier_freq;
    let sample_rate = state.config.lock().unwrap().sample_rate;

    // Encode the entire message upfront
    let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
    let samples = encoder.encode(&text);

    if samples.is_empty() {
        return Err("Nothing to transmit".into());
    }

    // Reset abort flag
    let abort = state.tx_abort.clone();
    abort.store(false, Ordering::SeqCst);

    // Verify DATA mode and set TX power (both non-fatal; auto-disconnects on serial error)
    let target_watts = state.config.lock().unwrap().tx_power_watts;
    let _ = with_radio(&state, &app, |radio| {
        ensure_data_mode(radio.as_mut());
        if let Err(e) = radio.set_tx_power(target_watts) {
            log::warn!("TX power set failed (continuing): {e}");
        }
        Ok(())
    });

    // Shared playback position for progress tracking
    let play_pos = Arc::new(AtomicUsize::new(0));
    let total_samples = samples.len();

    let handle = {
        let abort = abort.clone();
        let play_pos = play_pos.clone();

        thread::spawn(move || {
            run_tx_thread(app, abort, play_pos, samples, device_id, total_samples);
        })
    };

    state.tx_thread.lock().unwrap().replace(handle);

    Ok(())
}

fn stop_tx_inner(state: &AppState) -> Result<(), String> {
    state.tx_abort.store(true, Ordering::SeqCst);

    if let Some(handle) = state.tx_thread.lock().unwrap().take() {
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
    validate_tx_start(state.tx_thread.lock().unwrap().is_some())?;

    let carrier_freq = state.config.lock().unwrap().carrier_freq;
    let sample_rate = state.config.lock().unwrap().sample_rate;

    let abort = state.tx_abort.clone();
    abort.store(false, Ordering::SeqCst);

    // Tune always uses 10W regardless of the configured TX power setting
    let _ = with_radio(&state, &app, |radio| {
        ensure_data_mode(radio.as_mut());
        if let Err(e) = radio.set_tx_power(10) {
            log::warn!("TX power set failed (continuing): {e}");
        }
        Ok(())
    });

    let handle = thread::spawn(move || {
        run_tune_thread(app, abort, device_id, carrier_freq, f64::from(sample_rate));
    });

    state.tx_thread.lock().unwrap().replace(handle);
    Ok(())
}

fn stop_tune_inner(state: &AppState) -> Result<(), String> {
    state.tx_abort.store(true, Ordering::SeqCst);

    if let Some(handle) = state.tx_thread.lock().unwrap().take() {
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
                // Fill with silence — we're done
                for sample in output_buf.iter_mut() {
                    *sample = 0.0;
                }
                done_for_callback.store(true, Ordering::SeqCst);
                return;
            }

            let copy_len = output_buf.len().min(remaining);
            output_buf[..copy_len]
                .copy_from_slice(&samples_for_callback[current_pos..current_pos + copy_len]);

            // Fill any remaining buffer with silence
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

            // PTT OFF — deactivate before returning
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
            // Brief wait for the audio device to clock out its current buffer
            thread::sleep(Duration::from_millis(30));
            let _ = audio_output.stop();

            // Emit complete BEFORE PTT OFF — UI resets with zero IPC latency.
            // The frontend onComplete handler needs no follow-up invoke() call
            // because we self-clear the thread handle here with try_lock.
            let _ = app.emit(
                "tx-status",
                TxStatusPayload {
                    status: "complete".into(),
                    progress: 1.0,
                },
            );

            // Self-clear our handle from AppState so start_tx works immediately.
            // Use try_lock to avoid deadlock if stop_tx holds the lock concurrently
            // (in that case stop_tx will clear the handle itself via join).
            if let Ok(mut guard) = radio_state.tx_thread.try_lock() {
                let _ = guard.take();
            }

            // PTT OFF after the emit — audio is already silent, holding key
            // for ~50–100ms more is harmless for PSK-31.
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
    // validate_tx_start
    // -----------------------------------------------------------------------

    #[test]
    fn validate_tx_start_ok_when_not_transmitting() {
        assert!(validate_tx_start(false).is_ok());
    }

    #[test]
    fn validate_tx_start_err_when_already_transmitting() {
        let err = validate_tx_start(true).unwrap_err();
        assert_eq!(err, "Already transmitting");
    }

    // -----------------------------------------------------------------------
    // ensure_data_mode — uses an inline configurable mock
    // -----------------------------------------------------------------------

    /// Controls what the mock radio returns for get_frequency / get_mode /
    /// set_mode so we can exercise every branch of ensure_data_mode.
    struct ModeMock {
        freq_hz: f64,
        current_mode: String,
        /// If Some, get_frequency() returns this error instead of freq_hz
        freq_err: Option<String>,
        /// If Some, get_mode() returns this error instead of current_mode
        mode_err: Option<String>,
        /// If Some, set_mode() returns this error
        set_mode_err: Option<String>,
        /// Records the last mode passed to set_mode()
        set_mode_called_with: Option<String>,
    }

    impl ModeMock {
        /// Radio on 20m in the correct DATA-USB mode already
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

        /// Radio on 20m but in plain USB — needs correction to DATA-USB
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

        /// get_frequency() will fail
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

        /// get_mode() will fail
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

        /// set_mode() will fail
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
        // set_mode should NOT have been called
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
        // Non-fatal — should return without panicking and without calling set_mode
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
        // set_mode fails — ensure_data_mode should return without panicking
        let mut mock = ModeMock::set_mode_error();
        ensure_data_mode(&mut mock); // must not panic
        // set_mode was attempted
        assert_eq!(mock.set_mode_called_with.as_deref(), Some("DATA-USB"));
    }

    // -----------------------------------------------------------------------
    // stop_tx_inner
    // -----------------------------------------------------------------------

    #[test]
    fn stop_tx_inner_no_thread_no_radio_ok() {
        let state = AppState::new();
        // No thread, no radio — should just set abort flag and return Ok
        stop_tx_inner(&state).unwrap();
        assert!(state.tx_abort.load(Ordering::SeqCst), "abort flag must be set");
    }

    #[test]
    fn stop_tx_inner_calls_ptt_off_when_radio_present() {
        let state = AppState::new();
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        // ptt_off() is called inside stop_tx_inner; MockRadio accepts it without error
        stop_tx_inner(&state).unwrap();
    }

    #[test]
    fn stop_tx_inner_joins_thread() {
        let state = AppState::new();
        let handle = thread::spawn(|| {});
        *state.tx_thread.lock().unwrap() = Some(handle);
        stop_tx_inner(&state).unwrap();
        // Thread handle was consumed
        assert!(state.tx_thread.lock().unwrap().is_none());
    }

    // -----------------------------------------------------------------------
    // stop_tune_inner
    // -----------------------------------------------------------------------

    #[test]
    fn stop_tune_inner_no_thread_no_radio_ok() {
        let state = AppState::new();
        stop_tune_inner(&state).unwrap();
        assert!(state.tx_abort.load(Ordering::SeqCst));
    }

    #[test]
    fn stop_tune_inner_calls_ptt_off_and_restores_tx_power() {
        let state = AppState::new();
        state.config.lock().unwrap().tx_power_watts = 50;
        // MockRadio accepts ptt_off() and set_tx_power() without error
        *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
        stop_tune_inner(&state).unwrap();
        // Verify the inner function covered the radio-present branch (no panic = pass)
    }

    #[test]
    fn stop_tune_inner_joins_thread() {
        let state = AppState::new();
        let handle = thread::spawn(|| {});
        *state.tx_thread.lock().unwrap() = Some(handle);
        stop_tune_inner(&state).unwrap();
        assert!(state.tx_thread.lock().unwrap().is_none());
    }

    #[test]
    fn ensure_data_mode_lsb_below_10mhz() {
        // 40m — expects DATA-LSB
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
}
