//! CPAL audio adapters — implements AudioInput and AudioOutput using the cpal crate
//!
//! Think of cpal like Python's `sounddevice` library: it talks to the OS audio
//! system (CoreAudio on macOS, WASAPI on Windows, ALSA on Linux) and gives you
//! raw audio samples via callbacks.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::domain::{AudioDeviceInfo, AudioSample, Psk31Error, Psk31Result};
use crate::ports::{AudioInput, AudioOutput};

// ---------------------------------------------------------------------------
// Device enumeration (shared by AudioInput and AudioOutput)
// ---------------------------------------------------------------------------

/// Enumerate all audio devices and detect input/output capability.
///
/// Uses `host.output_devices()` as the authoritative source for output
/// capability. On macOS CoreAudio, `default_output_config()` fails for some
/// USB devices (e.g. USB Audio CODEC) even though they are valid outputs and
/// appear correctly in `output_devices()`. We fall back to
/// `default_output_config()` as a secondary check to catch any edge cases.
///
/// `output_unverified` is set when a device passes the `default_output_config()`
/// check but is NOT in `output_devices()` — an unusual edge case that warrants
/// a separate UI group.
fn enumerate_devices() -> crate::domain::Psk31Result<Vec<AudioDeviceInfo>> {
    let host = cpal::default_host();
    let default_input_name = host.default_input_device().and_then(|d| d.name().ok());
    let default_output_name = host.default_output_device().and_then(|d| d.name().ok());

    // Build set of confirmed output device names via output_devices() iterator.
    // This correctly includes USB Audio CODEC on macOS where default_output_config() fails.
    let confirmed_output_names: std::collections::HashSet<String> = host
        .output_devices()
        .map(|iter| iter.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();

    // Use a HashMap keyed by name so duplicate entries (macOS exposes some USB duplex
    // devices twice — once for the input stream, once for the output stream) get
    // merged rather than dropped.
    let mut device_map: std::collections::HashMap<String, AudioDeviceInfo> =
        std::collections::HashMap::new();
    let mut insertion_order: Vec<String> = Vec::new();

    if let Ok(all) = host.devices() {
        for device in all {
            let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            let is_input = device.default_input_config().is_ok();
            let confirmed_out = confirmed_output_names.contains(&name);
            let is_default = default_input_name.as_deref() == Some(name.as_str())
                || default_output_name.as_deref() == Some(name.as_str());

            if let Some(existing) = device_map.get_mut(&name) {
                // Merge: OR the input flag in case we saw the output entry first
                existing.is_input |= is_input;
            } else {
                insertion_order.push(name.clone());
                device_map.insert(name.clone(), AudioDeviceInfo {
                    id: name.clone(),
                    name,
                    is_input,
                    is_output: true, // all devices offered as output candidates (see module doc)
                    is_default,
                    output_unverified: !confirmed_out,
                });
            }
        }
    }

    let devices: Vec<AudioDeviceInfo> = insertion_order
        .into_iter()
        .filter_map(|name| device_map.remove(&name))
        .collect();

    Ok(devices)
}

// ---------------------------------------------------------------------------
// AudioInput
// ---------------------------------------------------------------------------

/// Audio input adapter backed by cpal.
///
/// Important: `cpal::Stream` is `!Send` — it can only live on the thread that
/// created it. That's why we don't store CpalAudioInput in AppState. Instead,
/// the audio commands spawn a dedicated thread that owns this struct.
pub struct CpalAudioInput {
    stream: Option<Stream>,
    running: Arc<AtomicBool>,
}

impl CpalAudioInput {
    pub fn new() -> Self {
        Self {
            stream: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl AudioInput for CpalAudioInput {
    fn list_devices(&self) -> Psk31Result<Vec<AudioDeviceInfo>> {
        enumerate_devices()
    }

    fn start(
        &mut self,
        device_id: &str,
        mut callback: Box<dyn FnMut(&[AudioSample]) + Send + 'static>,
    ) -> Psk31Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(Psk31Error::Audio("Audio stream already running".into()));
        }

        let host = cpal::default_host();

        // Find the requested device by name
        let device = host
            .input_devices()
            .map_err(|e| Psk31Error::Audio(format!("Failed to enumerate devices: {e}")))?
            .find(|d| d.name().map(|n| n == device_id).unwrap_or(false))
            .ok_or_else(|| {
                Psk31Error::Audio(format!("Audio device not found: {device_id}"))
            })?;

        // Configure for 48 kHz mono f32 — standard for ham radio digital modes
        let config = StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(48000),
            buffer_size: cpal::BufferSize::Default,
        };

        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        let err_running = self.running.clone();

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    callback(data);
                },
                move |err| {
                    log::error!("Audio stream error: {err}");
                    err_running.store(false, Ordering::SeqCst);
                },
                None, // No timeout
            )
            .map_err(|e| Psk31Error::Audio(format!("Failed to build stream: {e}")))?;

        stream
            .play()
            .map_err(|e| Psk31Error::Audio(format!("Failed to start stream: {e}")))?;

        self.stream = Some(stream);

        Ok(())
    }

    fn stop(&mut self) -> Psk31Result<()> {
        self.running.store(false, Ordering::SeqCst);
        // Dropping the stream stops capture
        self.stream = None;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// AudioOutput
// ---------------------------------------------------------------------------

/// Audio output adapter backed by cpal.
///
/// Same `!Send` constraint as CpalAudioInput — lives on a dedicated TX thread.
/// The callback is `FnMut(&mut [f32])`: cpal hands you a buffer, you fill it
/// with samples. If you have nothing to play, fill with zeros (silence).
pub struct CpalAudioOutput {
    stream: Option<Stream>,
    running: Arc<AtomicBool>,
}

impl CpalAudioOutput {
    pub fn new() -> Self {
        Self {
            stream: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl AudioOutput for CpalAudioOutput {
    fn list_devices(&self) -> Psk31Result<Vec<AudioDeviceInfo>> {
        enumerate_devices()
    }

    fn start(
        &mut self,
        device_id: &str,
        mut callback: Box<dyn FnMut(&mut [AudioSample]) + Send + 'static>,
    ) -> Psk31Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(Psk31Error::Audio("Audio output already running".into()));
        }

        let host = cpal::default_host();

        let device = host
            .devices()
            .map_err(|e| Psk31Error::Audio(format!("Failed to enumerate devices: {e}")))?
            .find(|d| d.name().map(|n| n == device_id).unwrap_or(false))
            .ok_or_else(|| {
                Psk31Error::Audio(format!("Audio output device not found: {device_id}"))
            })?;

        let config = StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(48000),
            buffer_size: cpal::BufferSize::Default,
        };

        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        let err_running = self.running.clone();

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    callback(data);
                },
                move |err| {
                    log::error!("Audio output error: {err}");
                    err_running.store(false, Ordering::SeqCst);
                },
                None,
            )
            .map_err(|e| Psk31Error::Audio(format!("Failed to build output stream: {e}")))?;

        stream
            .play()
            .map_err(|e| Psk31Error::Audio(format!("Failed to start output stream: {e}")))?;

        self.stream = Some(stream);

        Ok(())
    }

    fn stop(&mut self) -> Psk31Result<()> {
        self.running.store(false, Ordering::SeqCst);
        self.stream = None;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- AudioInput tests ---

    #[test]
    fn test_new_not_running() {
        let input = CpalAudioInput::new();
        assert!(!input.is_running());
    }

    #[test]
    fn test_list_devices_ok() {
        let input = CpalAudioInput::new();
        let result = input.list_devices();
        assert!(result.is_ok());
    }


    #[test]
    fn test_stop_idempotent() {
        let mut input = CpalAudioInput::new();
        assert!(input.stop().is_ok());
        assert!(input.stop().is_ok());
    }

    #[test]
    fn test_start_bad_device_errors() {
        let mut input = CpalAudioInput::new();
        let result = input.start(
            "nonexistent-device-that-does-not-exist",
            Box::new(|_samples| {}),
        );
        assert!(result.is_err());
    }

    // --- AudioOutput tests ---

    #[test]
    fn test_output_new_not_running() {
        let output = CpalAudioOutput::new();
        assert!(!output.is_running());
    }

    #[test]
    fn test_output_stop_idempotent() {
        let mut output = CpalAudioOutput::new();
        assert!(output.stop().is_ok());
        assert!(output.stop().is_ok());
    }
}
