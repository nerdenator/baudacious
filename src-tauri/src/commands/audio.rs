//! Audio commands — list devices, start/stop FFT streaming, RX decoding
//!
//! Architecture: `cpal::Stream` is `!Send` (like a Python object bound to one thread).
//! So we can't store it in AppState behind a Mutex. Instead, `start_audio_stream`
//! spawns a dedicated audio thread that owns the CpalAudioInput and ring buffer.
//! AppState only holds an AtomicBool shutdown flag and the thread's JoinHandle.
//!
//! The RX decoder runs inside the same audio thread — when `rx_running` is true,
//! each audio sample is fed to the Psk31Decoder alongside FFT processing.

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::adapters::cpal_audio::CpalAudioInput;
use crate::domain::AudioDeviceInfo;
use crate::dsp::fft::FftProcessor;
use crate::modem::decoder::Psk31Decoder;
use crate::ports::AudioInput;
use crate::state::AppState;

/// Payload for the `fft-data` event sent to the frontend
#[derive(Clone, Serialize)]
struct FftPayload {
    magnitudes: Vec<f32>,
}

/// Payload for the `audio-status` event
#[derive(Clone, Serialize)]
struct AudioStatusPayload {
    status: String,
}

/// Payload for the `rx-text` event — decoded characters from the RX decoder
#[derive(Clone, Serialize)]
struct RxTextPayload {
    text: String,
}

/// Payload for the `signal-level` event — normalized AGC-derived signal strength
#[derive(Clone, Serialize)]
struct SignalLevelPayload {
    level: f32,
}

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    let input = CpalAudioInput::new();
    input.list_devices().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_audio_stream(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    // Check if already running
    if state.audio_running.load(Ordering::SeqCst) {
        return Err("Audio stream already running".into());
    }

    let running = state.audio_running.clone();
    running.store(true, Ordering::SeqCst);

    if let Err(e) = state
        .audio_device_name
        .lock()
        .map(|mut name| *name = Some(device_id.clone()))
        .map_err(|_| "Audio state corrupted".to_string())
    {
        running.store(false, Ordering::SeqCst);
        let _ = app.emit("audio-status", AudioStatusPayload {
            status: format!("error: {e}"),
        });
        return Err(e);
    }

    let rx_running = state.rx_running.clone();
    let rx_carrier_freq = state.rx_carrier_freq.clone();
    let audio_device_name = state.audio_device_name.clone();
    let sample_rate = state.config.lock().unwrap().sample_rate;

    let handle = thread::spawn(move || {
        run_audio_thread(app, running, rx_running, rx_carrier_freq, audio_device_name, device_id, sample_rate);
    });

    state
        .audio_thread
        .lock()
        .unwrap()
        .replace(handle);

    Ok(())
}

#[tauri::command]
pub fn stop_audio_stream(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Join the thread — it clears audio_device_name on exit
    stop_audio_stream_inner(&state.audio_running, &state.rx_running, &state.audio_thread)
}

fn start_rx_inner(audio_running: &AtomicBool, rx_running: &AtomicBool) -> Result<(), String> {
    if !audio_running.load(Ordering::SeqCst) {
        return Err("Audio stream not running. Start audio first.".into());
    }
    rx_running.store(true, Ordering::SeqCst);
    Ok(())
}

fn stop_rx_inner(rx_running: &AtomicBool) -> Result<(), String> {
    rx_running.store(false, Ordering::SeqCst);
    Ok(())
}

fn set_carrier_frequency_inner(
    rx_carrier_freq: &std::sync::Mutex<f64>,
    config: &std::sync::Mutex<crate::domain::ModemConfig>,
    freq_hz: f64,
) -> Result<(), String> {
    if !(200.0..=3500.0).contains(&freq_hz) {
        return Err("Carrier frequency must be between 200-3500 Hz".into());
    }
    *rx_carrier_freq.lock().unwrap() = freq_hz;
    config.lock().unwrap().carrier_freq = freq_hz;
    Ok(())
}

fn stop_audio_stream_inner(
    audio_running: &AtomicBool,
    rx_running: &AtomicBool,
    audio_thread: &std::sync::Mutex<Option<thread::JoinHandle<()>>>,
) -> Result<(), String> {
    rx_running.store(false, Ordering::SeqCst);
    audio_running.store(false, Ordering::SeqCst);
    if let Some(handle) = audio_thread.lock().unwrap().take() {
        handle.join().map_err(|_| "Audio thread panicked".to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn start_rx(state: tauri::State<'_, AppState>) -> Result<(), String> {
    start_rx_inner(&state.audio_running, &state.rx_running)
}

#[tauri::command]
pub fn stop_rx(state: tauri::State<'_, AppState>) -> Result<(), String> {
    stop_rx_inner(&state.rx_running)
}

#[tauri::command]
pub fn set_carrier_frequency(
    state: tauri::State<'_, AppState>,
    freq_hz: f64,
) -> Result<(), String> {
    set_carrier_frequency_inner(&state.rx_carrier_freq, &state.config, freq_hz)
}

/// The main audio processing loop, runs on its own thread.
///
/// Flow: cpal callback → ring buffer → DSP loop → FFT + RX decoder → emit events
fn run_audio_thread(
    app: AppHandle,
    running: Arc<AtomicBool>,
    rx_running: Arc<AtomicBool>,
    rx_carrier_freq: Arc<Mutex<f64>>,
    audio_device_name: Arc<Mutex<Option<String>>>,
    device_id: String,
    sample_rate: u32,
) {
    // Emit status
    let _ = app.emit("audio-status", AudioStatusPayload { status: "running".into() });

    // Create ring buffer — 8192 samples gives ~170ms buffer at 48kHz
    // Think of it like a Python `collections.deque(maxlen=8192)` but lock-free
    let rb = HeapRb::<f32>::new(8192);
    let (mut producer, mut consumer) = rb.split();

    // Create audio input and start capture
    let mut audio_input = CpalAudioInput::new();
    let capture_result = audio_input.start(
        &device_id,
        Box::new(move |samples: &[f32]| {
            // Push samples into ring buffer; try_push drops the newest sample if full
            // (oldest data stays). For real-time audio, push_overwrite (drop oldest)
            // would be preferable — consider switching if latency becomes an issue.
            for &sample in samples {
                let _ = producer.try_push(sample);
            }
        }),
    );

    if let Err(e) = capture_result {
        log::error!("Failed to start audio capture: {e}");
        running.store(false, Ordering::SeqCst);
        let _ = app.emit("audio-status", AudioStatusPayload {
            status: format!("error: {e}"),
        });
        return;
    }

    // DSP loop: pull samples, compute FFT + RX decode, emit to frontend
    let fft_size = 4096;
    let hop_size = 2048; // 50% overlap for smooth waterfall scrolling
    let mut fft = FftProcessor::new(fft_size);
    let mut sample_buf: Vec<f32> = Vec::with_capacity(fft_size);

    // RX decoder — created with configured sample rate and initial carrier freq
    let initial_carrier = *rx_carrier_freq.lock().unwrap();
    let mut decoder = Psk31Decoder::new(initial_carrier, sample_rate);

    // Buffer decoded chars to emit in batches (reduces event overhead)
    let mut rx_text_buf = String::new();

    // Throttle signal-level events to ~500ms (100 iterations × 5ms sleep)
    let mut signal_emit_counter: u32 = 0;

    // Set when cpal error callback fires (device removed mid-stream)
    let mut device_lost = false;

    while running.load(Ordering::SeqCst) {
        // Check if cpal silently killed the stream (e.g. USB device removed)
        if !audio_input.is_running() {
            running.store(false, Ordering::SeqCst);
            device_lost = true;
            break;
        }
        // Drain available samples from ring buffer into a temporary vec
        // so we can use them for both FFT and RX decoding
        let mut new_samples: Vec<f32> = Vec::new();
        while let Some(sample) = consumer.try_pop() {
            new_samples.push(sample);
        }

        // RX decoding: feed every new sample to the decoder when enabled
        if rx_running.load(Ordering::SeqCst) {
            // Check if carrier frequency changed (click-to-tune)
            decoder.update_carrier_if_changed(*rx_carrier_freq.lock().unwrap());

            for &sample in &new_samples {
                if let Some(ch) = decoder.process(sample) {
                    rx_text_buf.push(ch);
                }
            }

            // Emit any decoded text as a batch
            if !rx_text_buf.is_empty() {
                let _ = app.emit("rx-text", RxTextPayload { text: rx_text_buf.clone() });
                rx_text_buf.clear();
            }
        }

        // Accumulate samples for FFT processing
        sample_buf.extend_from_slice(&new_samples);

        // When we have enough samples, compute FFT with 50% overlap
        while sample_buf.len() >= fft_size {
            let magnitudes = fft.compute(&sample_buf[..fft_size]);
            let _ = app.emit("fft-data", FftPayload { magnitudes });

            // Advance by hop_size (keep the overlap portion)
            sample_buf.drain(..hop_size);
        }

        // Emit signal level every ~500ms (100 iterations)
        signal_emit_counter += 1;
        if signal_emit_counter >= 100 {
            signal_emit_counter = 0;
            let level = if rx_running.load(Ordering::Relaxed) {
                decoder.signal_strength()
            } else {
                0.0
            };
            let _ = app.emit("signal-level", SignalLevelPayload { level });
        }

        // Sleep to avoid busy-waiting (~5ms = well within 42ms frame budget)
        thread::sleep(Duration::from_millis(5));
    }

    // Clean shutdown
    let _ = audio_input.stop();
    *audio_device_name.lock().unwrap() = None;

    let status = if device_lost {
        "error: audio device lost".to_string()
    } else {
        "stopped".to_string()
    };
    let _ = app.emit("audio-status", AudioStatusPayload { status });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    // --- start_rx_inner ---

    #[test]
    fn start_rx_inner_err_when_audio_not_running() {
        let audio = AtomicBool::new(false);
        let rx = AtomicBool::new(false);
        let err = start_rx_inner(&audio, &rx).unwrap_err();
        assert!(err.contains("Audio stream not running"));
        assert!(!rx.load(Ordering::SeqCst), "rx_running must stay false");
    }

    #[test]
    fn start_rx_inner_ok_when_audio_running() {
        let audio = AtomicBool::new(true);
        let rx = AtomicBool::new(false);
        start_rx_inner(&audio, &rx).unwrap();
        assert!(rx.load(Ordering::SeqCst), "rx_running must be set to true");
    }

    // --- stop_rx_inner ---

    #[test]
    fn stop_rx_inner_clears_rx_running() {
        let rx = AtomicBool::new(true);
        stop_rx_inner(&rx).unwrap();
        assert!(!rx.load(Ordering::SeqCst));
    }

    // --- set_carrier_frequency_inner ---

    #[test]
    fn set_carrier_frequency_inner_rejects_below_200hz() {
        let freq = std::sync::Mutex::new(1000.0_f64);
        let config = std::sync::Mutex::new(crate::domain::ModemConfig::default());
        let err = set_carrier_frequency_inner(&freq, &config, 199.9).unwrap_err();
        assert!(err.contains("200-3500"));
    }

    #[test]
    fn set_carrier_frequency_inner_rejects_above_3500hz() {
        let freq = std::sync::Mutex::new(1000.0_f64);
        let config = std::sync::Mutex::new(crate::domain::ModemConfig::default());
        let err = set_carrier_frequency_inner(&freq, &config, 3500.1).unwrap_err();
        assert!(err.contains("200-3500"));
    }

    #[test]
    fn set_carrier_frequency_inner_updates_both_stores() {
        let freq = std::sync::Mutex::new(1000.0_f64);
        let config = std::sync::Mutex::new(crate::domain::ModemConfig::default());
        set_carrier_frequency_inner(&freq, &config, 1500.0).unwrap();
        assert_eq!(*freq.lock().unwrap(), 1500.0);
        assert_eq!(config.lock().unwrap().carrier_freq, 1500.0);
    }

    #[test]
    fn set_carrier_frequency_inner_accepts_boundary_values() {
        let freq = std::sync::Mutex::new(1000.0_f64);
        let config = std::sync::Mutex::new(crate::domain::ModemConfig::default());
        assert!(set_carrier_frequency_inner(&freq, &config, 200.0).is_ok());
        assert!(set_carrier_frequency_inner(&freq, &config, 3500.0).is_ok());
    }

    // --- stop_audio_stream_inner ---

    #[test]
    fn stop_audio_stream_inner_clears_flags_when_no_thread() {
        let audio = AtomicBool::new(true);
        let rx = AtomicBool::new(true);
        let thread_slot: std::sync::Mutex<Option<thread::JoinHandle<()>>> =
            std::sync::Mutex::new(None);
        stop_audio_stream_inner(&audio, &rx, &thread_slot).unwrap();
        assert!(!audio.load(Ordering::SeqCst));
        assert!(!rx.load(Ordering::SeqCst));
    }

    #[test]
    fn stop_audio_stream_inner_joins_thread() {
        let audio = AtomicBool::new(true);
        let rx = AtomicBool::new(false);
        let handle = thread::spawn(|| {});
        let thread_slot = std::sync::Mutex::new(Some(handle));
        stop_audio_stream_inner(&audio, &rx, &thread_slot).unwrap();
        assert!(thread_slot.lock().unwrap().is_none());
    }
}
