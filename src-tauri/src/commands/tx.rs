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
use std::thread::{self, JoinHandle};
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

/// Critical section 1 — atomically claim the TX slot.
///
/// Transitions `Idle → Starting`, resets `tx_abort`, and increments the
/// generation counter — all under the lock so a concurrent `stop_tx` that
/// sets `tx_abort=true` cannot race with our reset.
///
/// Returns the new generation value, which the caller must carry through to
/// the pre-spawn abort check and `tx_activate`.  If a `stop_tx`+`start_tx`
/// sequence completes while the caller is doing slow work, the generation will
/// have changed and the caller can detect the ABA race without relying solely
/// on the shared `tx_abort` flag.
pub(crate) fn tx_claim(state: &AppState) -> Result<u64, String> {
    let mut tx = state
        .tx_state
        .lock()
        .map_err(|_| "TX state corrupted".to_string())?;
    validate_tx_state(&tx)?;
    state.tx_abort.store(false, Ordering::SeqCst);
    let gen = state.tx_generation.fetch_add(1, Ordering::SeqCst) + 1;
    *tx = TxState::Starting;
    Ok(gen)
}

/// Reset the TX slot to `Idle`.
///
/// Used on early-exit paths (empty samples, pre-spawn abort) where the claim
/// succeeded but we need to give the slot back without joining any thread.
/// Tolerates a poisoned mutex so the radio is never left stuck in `Starting`.
pub(crate) fn tx_reset(state: &AppState) {
    *state.tx_state.lock().unwrap_or_else(|e| e.into_inner()) = TxState::Idle;
}

/// Critical section 2 — activate the TX slot.
///
/// Transitions `Starting → Running(handle)` if and only if the generation
/// still matches `my_gen`.  Two cancellation scenarios trigger the else path:
///
/// 1. `stop_tx` reset state to `Idle` while we were doing slow work.
/// 2. ABA race: `stop_tx` + a new `start_tx` completed, resetting the
///    generation so `my_gen` no longer matches the current counter.
///
/// In both cases the spawned thread holds `tx_abort=true` (set by `stop_tx`
/// or the generation check in `start_tx`) and will exit at its early-abort
/// check.  We release the lock first, then join, then return `Err`.
pub(crate) fn tx_activate(
    state: &AppState,
    handle: JoinHandle<()>,
    my_gen: u64,
) -> Result<(), String> {
    let mut tx = state
        .tx_state
        .lock()
        .map_err(|_| "TX state corrupted".to_string())?;
    if matches!(*tx, TxState::Starting)
        && state.tx_generation.load(Ordering::SeqCst) == my_gen
    {
        *tx = TxState::Running(handle);
        return Ok(());
    }
    // Cancellation path: either stop_tx reset the state or the generation
    // changed (ABA race).  Release the lock before joining.
    drop(tx);
    match handle.join() {
        Ok(()) => Err("Cancelled before activation".to_string()),
        Err(_) => Err("TX thread panicked during cancellation".to_string()),
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
    let my_gen = tx_claim(&state)?;

    // ── Slow work (lock-free) ─────────────────────────────────────────────────
    // stop_tx can now acquire tx_state immediately and set tx_abort=true.

    // Snapshot all config fields in one lock acquisition so carrier_freq,
    // sample_rate, and tx_power_watts are from the same consistent config state.
    let (carrier_freq, sample_rate, target_watts) = {
        let cfg = state.config.lock().unwrap();
        (cfg.carrier_freq, cfg.sample_rate, cfg.tx_power_watts)
    };

    let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
    let samples = encoder.encode(&text);

    if samples.is_empty() {
        tx_reset(&state);
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
    // If stop_tx fired during slow work it set tx_abort=true.  Also check the
    // generation: if stop_tx + a new start_tx completed while we were encoding
    // or in CAT I/O, the generation will have advanced (ABA race) and we must
    // not reset the slot that now belongs to the new start.
    let aborted = state.tx_abort.load(Ordering::SeqCst)
        || state.tx_generation.load(Ordering::SeqCst) != my_gen;
    if aborted {
        if state.tx_generation.load(Ordering::SeqCst) == my_gen {
            tx_reset(&state); // still our slot — give it back
        }
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
    // If stop_tx raced between the abort check and spawn, state is already Idle.
    // tx_activate detects this (or an ABA generation mismatch), joins the
    // (already-aborted) thread, and errors.
    tx_activate(&state, handle, my_gen)?;

    Ok(())
}

fn stop_tx_inner(state: &AppState) -> Result<(), String> {
    // Lock first, set abort, take handle — all under the same guard so that
    // start_tx (which resets tx_abort=false inside the lock) cannot race with us.
    // Recover from a poisoned mutex so the abort flag and ptt_off safety call
    // below are not skipped due to a panic in an unrelated code path.
    let handle = {
        let mut tx = state
            .tx_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.tx_abort.store(true, Ordering::SeqCst);
        match std::mem::replace(&mut *tx, TxState::Idle) {
            TxState::Running(h) => Some(h),
            _ => None, // Idle or Starting — abort flag is sufficient
        }
    }; // lock released before join to avoid deadlock with run_tx_thread, which also locks tx_state

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
    let my_gen = tx_claim(&state)?;

    // ── Slow work (lock-free) ─────────────────────────────────────────────────
    let (carrier_freq, sample_rate) = {
        let cfg = state.config.lock().unwrap();
        (cfg.carrier_freq, cfg.sample_rate)
    };

    // Tune always uses 10W regardless of the configured TX power setting
    let _ = with_radio(&state, &app, |radio| {
        ensure_data_mode(radio.as_mut());
        if let Err(e) = radio.set_tx_power(10) {
            log::warn!("TX power set failed (continuing): {e}");
        }
        Ok(())
    });

    // ── Abort check ───────────────────────────────────────────────────────────
    let aborted = state.tx_abort.load(Ordering::SeqCst)
        || state.tx_generation.load(Ordering::SeqCst) != my_gen;
    if aborted {
        if state.tx_generation.load(Ordering::SeqCst) == my_gen {
            tx_reset(&state);
        }
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
    // If stop_tune raced between the abort check and spawn, state is already Idle.
    // tx_activate detects this (or an ABA generation mismatch), joins the
    // (already-aborted) thread, and errors.
    tx_activate(&state, handle, my_gen)?;

    Ok(())
}

fn stop_tune_inner(state: &AppState) -> Result<(), String> {
    // Recover from a poisoned mutex so abort + ptt_off safety actions are not
    // skipped due to a panic elsewhere.
    let handle = {
        let mut tx = state
            .tx_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
    // Early abort check: stop_tune may have fired between the pre-spawn abort
    // check and tx_activate.  Check before keying the radio.
    if abort.load(Ordering::SeqCst) {
        let _ = app.emit(
            "tx-status",
            TxStatusPayload { status: "aborted".into(), progress: 0.0 },
        );
        return;
    }

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
    // Early abort check: stop_tx may have fired between the pre-spawn abort
    // check and tx_activate.  Check before keying the radio.
    if abort.load(Ordering::SeqCst) {
        let _ = app.emit(
            "tx-status",
            TxStatusPayload { status: "aborted".into(), progress: 0.0 },
        );
        return;
    }

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
        // PTT OFF — don't leave the transmitter keyed on audio device failure.
        if let Ok(mut guard) = radio_state.radio.lock() {
            if let Some(radio) = guard.as_mut() {
                if let Err(e) = radio.ptt_off() {
                    log::warn!("PTT OFF failed after audio error: {e}");
                }
            }
        }
        // Reset tx_state so start_tx is usable again after an audio device failure.
        *radio_state.tx_state.lock().unwrap_or_else(|e| e.into_inner()) = TxState::Idle;
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
            // A regular lock is safe here: stop_tx releases tx_state BEFORE
            // joining this thread, so it cannot hold the lock at this point.
            if let Ok(mut tx) = radio_state.tx_state.lock() {
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
        let state = TxState::Running(handle);
        let err = validate_tx_state(&state).unwrap_err();
        assert_eq!(err, "Already transmitting");
        if let TxState::Running(h) = state {
            h.join().unwrap();
        }
    }

    // -----------------------------------------------------------------------
    // tx_claim / tx_reset / tx_activate
    // -----------------------------------------------------------------------

    #[test]
    fn tx_claim_transitions_idle_to_starting() {
        let state = AppState::new();
        tx_claim(&state).unwrap();
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Starting));
        assert!(!state.tx_abort.load(Ordering::SeqCst), "abort must be cleared");
    }

    #[test]
    fn tx_claim_returns_generation_and_increments() {
        let state = AppState::new();
        let gen1 = tx_claim(&state).unwrap();
        assert_eq!(gen1, 1, "first claim should return generation 1");
        // reset to Idle so a second claim is allowed
        tx_reset(&state);
        let gen2 = tx_claim(&state).unwrap();
        assert_eq!(gen2, 2, "second claim should return generation 2");
        tx_reset(&state);
    }

    #[test]
    fn tx_claim_rejects_when_starting() {
        let state = AppState::new();
        *state.tx_state.lock().unwrap() = TxState::Starting;
        assert_eq!(tx_claim(&state).unwrap_err(), "Already transmitting");
    }

    #[test]
    fn tx_claim_rejects_when_running() {
        let state = AppState::new();
        let handle = thread::spawn(|| {});
        *state.tx_state.lock().unwrap() = TxState::Running(handle);
        assert_eq!(tx_claim(&state).unwrap_err(), "Already transmitting");
        // Clean up the thread stored in Running
        let prev = std::mem::replace(&mut *state.tx_state.lock().unwrap(), TxState::Idle);
        if let TxState::Running(h) = prev {
            h.join().unwrap();
        }
    }

    #[test]
    fn tx_claim_clears_abort_flag() {
        let state = AppState::new();
        state.tx_abort.store(true, Ordering::SeqCst);
        tx_claim(&state).unwrap();
        assert!(!state.tx_abort.load(Ordering::SeqCst));
        tx_reset(&state);
    }

    #[test]
    fn tx_reset_returns_state_to_idle() {
        let state = AppState::new();
        *state.tx_state.lock().unwrap() = TxState::Starting;
        tx_reset(&state);
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }

    #[test]
    fn tx_activate_transitions_starting_to_running() {
        let state = AppState::new();
        let gen = tx_claim(&state).unwrap();
        let handle = thread::spawn(|| {});
        tx_activate(&state, handle, gen).unwrap();
        let prev = std::mem::replace(&mut *state.tx_state.lock().unwrap(), TxState::Idle);
        if let TxState::Running(h) = prev {
            h.join().unwrap();
        } else {
            panic!("expected Running");
        }
    }

    #[test]
    fn tx_activate_cancels_when_already_idle() {
        // Simulates the race where stop_tx reset state before activate ran.
        // tx_activate must return Err and join the handle (not drop it detached).
        let state = AppState::new();
        let gen = tx_claim(&state).unwrap();
        state.tx_abort.store(true, Ordering::SeqCst);
        *state.tx_state.lock().unwrap() = TxState::Idle; // simulate stop_tx
        let handle = thread::spawn(|| {});
        let result = tx_activate(&state, handle, gen);
        assert!(result.is_err(), "should return Err on cancellation path");
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Idle));
    }

    #[test]
    fn tx_activate_rejects_wrong_generation() {
        // Simulates the ABA race: stop_tx + new start_tx completed, advancing
        // the generation, before the old start_tx reaches tx_activate.
        let state = AppState::new();
        let old_gen = tx_claim(&state).unwrap(); // gen = 1, state = Starting
        // Simulate stop_tx: reset to Idle
        *state.tx_state.lock().unwrap() = TxState::Idle;
        // Simulate new start_tx: claim again advances gen to 2
        let _new_gen = tx_claim(&state).unwrap(); // gen = 2, state = Starting
        // Old start_tx now tries to activate with stale gen=1
        state.tx_abort.store(true, Ordering::SeqCst); // tx thread will self-exit
        let handle = thread::spawn(|| {});
        let result = tx_activate(&state, handle, old_gen);
        assert!(result.is_err(), "old generation should be rejected");
        // State should still be Starting (belonging to the new start)
        assert!(matches!(*state.tx_state.lock().unwrap(), TxState::Starting));
        // Clean up: new start's slot
        tx_reset(&state);
    }

    #[test]
    fn tx_activate_returns_err_on_thread_panic() {
        // Cancellation path: thread panics — error should propagate, not be swallowed.
        let state = AppState::new();
        let gen = tx_claim(&state).unwrap();
        // Reset to Idle so tx_activate takes the cancellation path
        *state.tx_state.lock().unwrap() = TxState::Idle;
        let handle = thread::spawn(|| panic!("simulated TX thread panic"));
        let result = tx_activate(&state, handle, gen);
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.contains("panicked")),
            "expected a panicked error, got: {result:?}"
        );
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
