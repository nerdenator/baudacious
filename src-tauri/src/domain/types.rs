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
    pub is_output: bool,
    pub is_default: bool,
    /// true = device is in host.output_devices() but default_output_config() fails
    /// (e.g. USB Audio CODEC on macOS CoreAudio)
    pub output_unverified: bool,
}

/// Information about a serial port
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortInfo {
    pub name: String,
    pub port_type: String,
    pub device_hint: Option<String>,
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

/// Comprehensive radio status returned from the IF; CAT command.
/// Carries all the information needed to sync the UI on connect or poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioStatus {
    pub frequency_hz: u64,
    pub mode: String,
    pub is_transmitting: bool,
    pub rit_offset_hz: i32,
    pub rit_enabled: bool,
    pub split: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- Frequency constructors ---

    #[test]
    fn frequency_hz_constructor() {
        assert_eq!(Frequency::hz(14_070_000.0).as_hz(), 14_070_000.0);
    }

    #[test]
    fn frequency_khz_constructor() {
        assert_eq!(Frequency::khz(14_070.0).as_hz(), 14_070_000.0);
    }

    #[test]
    fn frequency_mhz_constructor() {
        assert_eq!(Frequency::mhz(14.070).as_hz(), 14_070_000.0);
    }

    #[test]
    fn frequency_equality() {
        assert_eq!(Frequency::khz(7035.0), Frequency::hz(7_035_000.0));
        assert_ne!(Frequency::hz(7_035_000.0), Frequency::hz(7_035_001.0));
    }

    // --- ModemConfig defaults ---

    #[test]
    fn modem_config_default_values() {
        let cfg = ModemConfig::default();
        assert_eq!(cfg.sample_rate, 48000);
        assert_eq!(cfg.carrier_freq, 1000.0);
        assert_eq!(cfg.fft_size, 4096);
        assert_eq!(cfg.tx_power_watts, 25);
    }

    #[test]
    fn modem_config_serde_default_tx_power() {
        // tx_power_watts has serde(default) — omitting it in JSON should produce 25
        let json = r#"{"sample_rate":48000,"carrier_freq":1000.0,"fft_size":4096}"#;
        let cfg: ModemConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.tx_power_watts, 25);
    }

    // --- ModemStatus defaults ---

    #[test]
    fn modem_status_default_values() {
        let status = ModemStatus::default();
        assert!(!status.rx_running);
        assert!(!status.tx_running);
        assert_eq!(status.carrier_freq_hz, 1000.0);
        assert_eq!(status.signal_level, 0.0);
    }

    // --- RadioStatus serialization and equality ---

    #[test]
    fn radio_status_partial_eq() {
        let a = RadioStatus {
            frequency_hz: 14_070_000,
            mode: "USB".into(),
            is_transmitting: false,
            rit_offset_hz: 0,
            rit_enabled: false,
            split: false,
        };
        let b = a.clone();
        assert_eq!(a, b);

        let c = RadioStatus { frequency_hz: 7_035_000, ..a.clone() };
        assert_ne!(a, c);
    }

    #[test]
    fn radio_status_serializes_to_camel_case() {
        let s = RadioStatus {
            frequency_hz: 14_070_000,
            mode: "USB".into(),
            is_transmitting: false,
            rit_offset_hz: 0,
            rit_enabled: false,
            split: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("frequencyHz"), "expected camelCase frequencyHz");
        assert!(json.contains("isTransmitting"), "expected camelCase isTransmitting");
    }

    // --- RadioInfo serialization ---

    #[test]
    fn radio_info_serializes_to_camel_case() {
        let info = RadioInfo {
            port: "/dev/tty.usbserial".into(),
            baud_rate: 38400,
            frequency_hz: 14_070_000.0,
            mode: "USB".into(),
            connected: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("frequencyHz"), "expected camelCase frequencyHz");
        assert!(json.contains("baudRate"), "expected camelCase baudRate");
    }

    // --- AudioDeviceInfo serialization ---

    #[test]
    fn audio_device_info_serializes_to_camel_case() {
        let dev = AudioDeviceInfo {
            id: "device-1".into(),
            name: "USB Audio".into(),
            is_input: true,
            is_output: false,
            is_default: true,
            output_unverified: false,
        };
        let json = serde_json::to_string(&dev).unwrap();
        assert!(json.contains("isInput"), "expected camelCase isInput");
        assert!(json.contains("isDefault"), "expected camelCase isDefault");
        assert!(json.contains("outputUnverified"), "expected camelCase outputUnverified");
    }
}
