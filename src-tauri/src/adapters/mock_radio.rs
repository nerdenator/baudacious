//! Mock radio adapter for development and testing without hardware.
//!
//! Activate by setting MOCK_RADIO=1 in the environment:
//!
//!   MOCK_RADIO=1 RUST_LOG=baudacious_lib=info npm run tauri dev
//!
//! Every RadioControl call is logged at INFO level so you can verify
//! exactly what the UI would send to a real radio.

use crate::domain::{Frequency, Psk31Result, RadioStatus};
use crate::ports::RadioControl;

/// Default frequency: 20m PSK-31 calling frequency
const DEFAULT_FREQ_HZ: f64 = 14_070_000.0;
/// Default mode: DATA-USB (standard for PSK-31)
const DEFAULT_MODE: &str = "DATA-USB";
/// Default TX power in watts
const DEFAULT_TX_POWER_W: u32 = 25;

pub struct MockRadio {
    frequency: f64,
    mode: String,
    tx_power: u32,
    is_transmitting: bool,
}

impl MockRadio {
    pub fn new() -> Self {
        log::info!(
            "[MOCK RADIO] Initialized at {:.3} MHz, mode={DEFAULT_MODE}, power={DEFAULT_TX_POWER_W}W",
            DEFAULT_FREQ_HZ / 1e6
        );
        Self {
            frequency: DEFAULT_FREQ_HZ,
            mode: DEFAULT_MODE.to_string(),
            tx_power: DEFAULT_TX_POWER_W,
            is_transmitting: false,
        }
    }
}

impl RadioControl for MockRadio {
    fn ptt_on(&mut self) -> Psk31Result<()> {
        self.is_transmitting = true;
        log::info!("[MOCK RADIO] PTT ON  → TX1;");
        Ok(())
    }

    fn ptt_off(&mut self) -> Psk31Result<()> {
        self.is_transmitting = false;
        log::info!("[MOCK RADIO] PTT OFF → TX0;");
        Ok(())
    }

    fn is_transmitting(&self) -> bool {
        self.is_transmitting
    }

    fn get_frequency(&mut self) -> Psk31Result<Frequency> {
        let hz = self.frequency as u64;
        log::info!(
            "[MOCK RADIO] GET FREQ → FA; → FA{hz:011};  ({:.3} MHz)",
            self.frequency / 1e6
        );
        Ok(Frequency::hz(self.frequency))
    }

    fn set_frequency(&mut self, freq: Frequency) -> Psk31Result<()> {
        let hz = freq.as_hz() as u64;
        log::info!(
            "[MOCK RADIO] SET FREQ → FA{hz:011};  ({:.3} MHz)",
            freq.as_hz() / 1e6
        );
        self.frequency = freq.as_hz();
        Ok(())
    }

    fn get_mode(&mut self) -> Psk31Result<String> {
        log::info!("[MOCK RADIO] GET MODE → MD0; → {}", self.mode);
        Ok(self.mode.clone())
    }

    fn set_mode(&mut self, mode: &str) -> Psk31Result<()> {
        log::info!("[MOCK RADIO] SET MODE → MD0?; → {mode}");
        self.mode = mode.to_string();
        Ok(())
    }

    fn get_tx_power(&mut self) -> Psk31Result<u32> {
        log::info!("[MOCK RADIO] GET TX POWER → PC; → PC{:03};  ({}W)", self.tx_power, self.tx_power);
        Ok(self.tx_power)
    }

    fn set_tx_power(&mut self, watts: u32) -> Psk31Result<()> {
        log::info!("[MOCK RADIO] SET TX POWER → PC{watts:03};  ({watts}W)");
        self.tx_power = watts;
        Ok(())
    }

    fn get_signal_strength(&mut self) -> Psk31Result<f32> {
        log::info!("[MOCK RADIO] GET S-METER → SM0; → SM00009;  (0.30 normalized)");
        Ok(0.3) // S3 approximately
    }

    fn get_status(&mut self) -> Psk31Result<RadioStatus> {
        log::info!(
            "[MOCK RADIO] GET STATUS → IF; → {:.3} MHz, mode={}",
            self.frequency / 1e6,
            self.mode
        );
        Ok(RadioStatus {
            frequency_hz: self.frequency as u64,
            mode: self.mode.clone(),
            is_transmitting: self.is_transmitting,
            rit_offset_hz: 0,
            rit_enabled: false,
            split: false,
        })
    }
}
