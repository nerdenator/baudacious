//! Core domain types

use serde::{Deserialize, Serialize};

/// Audio sample type (32-bit float, range -1.0 to 1.0)
pub type AudioSample = f32;

/// Frequency in Hz
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frequency(pub f64);

impl Frequency {
    pub fn hz(hz: f64) -> Self {
        Self(hz)
    }

    pub fn khz(khz: f64) -> Self {
        Self(khz * 1_000.0)
    }

    pub fn mhz(mhz: f64) -> Self {
        Self(mhz * 1_000_000.0)
    }

    pub fn as_hz(&self) -> f64 {
        self.0
    }
}

/// Information about an audio device
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_input: bool,
    pub is_default: bool,
}

/// Information about a serial port
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortInfo {
    pub name: String,
    pub port_type: String,
}

fn default_tx_power_watts() -> u32 {
    25
}

/// Modem configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModemConfig {
    /// Sample rate in Hz (typically 48000)
    pub sample_rate: u32,
    /// Audio carrier frequency in Hz (500-2500 typical)
    pub carrier_freq: f64,
    /// FFT size for waterfall display
    pub fft_size: usize,
    /// TX power in watts (applied before PTT ON)
    #[serde(default = "default_tx_power_watts")]
    pub tx_power_watts: u32,
}

impl Default for ModemConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            carrier_freq: 1000.0,
            fft_size: 4096,
            tx_power_watts: default_tx_power_watts(),
        }
    }
}

/// Current modem status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModemStatus {
    pub rx_running: bool,
    pub tx_running: bool,
    pub carrier_freq_hz: f64,
    pub signal_level: f32,
}

impl Default for ModemStatus {
    fn default() -> Self {
        Self {
            rx_running: false,
            tx_running: false,
            carrier_freq_hz: 1000.0,
            signal_level: 0.0,
        }
    }
}

/// Radio connection information returned after successful connect
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioInfo {
    pub port: String,
    pub baud_rate: u32,
    pub frequency_hz: f64,
    pub mode: String,
    pub connected: bool,
}
