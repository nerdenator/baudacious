//! Radio control port trait

use crate::domain::{Frequency, Psk31Result, RadioStatus};

/// Trait for radio control (PTT, frequency, mode, TX power)
pub trait RadioControl: Send {
    /// Engage PTT (start transmitting)
    fn ptt_on(&mut self) -> Psk31Result<()>;

    /// Release PTT (stop transmitting)
    fn ptt_off(&mut self) -> Psk31Result<()>;

    /// Check if PTT is currently engaged
    fn is_transmitting(&self) -> bool;

    /// Get current VFO frequency
    fn get_frequency(&mut self) -> Psk31Result<Frequency>;

    /// Set VFO frequency
    fn set_frequency(&mut self, freq: Frequency) -> Psk31Result<()>;

    /// Get current operating mode (e.g., "USB", "DATA-USB", "LSB")
    fn get_mode(&mut self) -> Psk31Result<String>;

    /// Set operating mode
    fn set_mode(&mut self, mode: &str) -> Psk31Result<()>;

    /// Get TX power in watts
    fn get_tx_power(&mut self) -> Psk31Result<u32>;

    /// Set TX power in watts
    fn set_tx_power(&mut self, watts: u32) -> Psk31Result<()>;

    /// Get S-meter signal strength, normalised 0.0–1.0 (from SM0 0–30 scale)
    fn get_signal_strength(&mut self) -> Psk31Result<f32>;

    /// Get comprehensive radio status via IF; (freq, mode, TX, RIT, split).
    /// Preferred over separate get_frequency + get_mode calls on connect.
    fn get_status(&mut self) -> Psk31Result<RadioStatus>;
}
