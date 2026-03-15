//! Configuration profiles
//!
//! A Configuration is a saved profile containing all settings for a particular
//! radio setup (audio devices, serial port, radio type, modem parameters).

use serde::{Deserialize, Serialize};

fn default_tx_power_watts() -> u32 {
    10
}

fn default_waterfall_palette() -> String {
    "classic".to_string()
}

fn default_waterfall_noise_floor() -> i32 {
    -100
}

fn default_waterfall_zoom() -> u32 {
    1
}

/// A saved configuration profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    /// Profile name (e.g., "FT-991A Home", "IC-7300 Portable")
    pub name: String,
    /// Selected audio input device ID
    pub audio_input: Option<String>,
    /// Selected audio output device ID
    pub audio_output: Option<String>,
    /// Selected serial port name
    pub serial_port: Option<String>,
    /// Serial baud rate
    pub baud_rate: u32,
    /// Radio type identifier (e.g., "FT-991A", "IC-7300")
    pub radio_type: String,
    /// Audio carrier frequency in Hz
    pub carrier_freq: f64,
    /// Waterfall color palette name ("classic", "heat", "viridis", "grayscale")
    #[serde(default = "default_waterfall_palette")]
    pub waterfall_palette: String,
    /// Waterfall noise floor in dBFS (e.g. -100)
    #[serde(default = "default_waterfall_noise_floor")]
    pub waterfall_noise_floor: i32,
    /// Waterfall zoom level (1, 2, or 4)
    #[serde(default = "default_waterfall_zoom")]
    pub waterfall_zoom: u32,
    /// TX power in watts applied before PTT ON (0–100)
    #[serde(default = "default_tx_power_watts")]
    pub tx_power_watts: u32,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            audio_input: None,
            audio_output: None,
            serial_port: None,
            baud_rate: 38400,
            radio_type: "FT-991A".to_string(),
            carrier_freq: 1000.0,
            waterfall_palette: default_waterfall_palette(),
            waterfall_noise_floor: default_waterfall_noise_floor(),
            waterfall_zoom: default_waterfall_zoom(),
            tx_power_watts: default_tx_power_watts(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_has_sensible_values() {
        let config = Configuration::default();
        assert_eq!(config.name, "Default");
        assert_eq!(config.baud_rate, 38400);
        assert_eq!(config.carrier_freq, 1000.0);
    }

    #[test]
    fn configuration_serializes_to_json() {
        let config = Configuration::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"name\":\"Default\""));
    }
}
