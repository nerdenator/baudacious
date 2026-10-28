//! FT-891 radio adapter using CAT (Computer Aided Transceiver) commands
//!
//! Same ASCII/`;` protocol as the FT-991A, so it reuses `CatSession`, with the
//! `Ft891` dialect for the replies that differ (`IF;` has no TX flag, `SM0;` is
//! 3 digits). Differences handled here:
//! - HF + 6m only (no 2m/70cm); 60m has no `BS;` code so it is set via `FA;` only
//! - TX power is 5–100 W
//! - Set commands have no reply per the CAT manual, so they are sent write-only
//! - PTT state comes from `TX;` (the `IF;` reply doesn't carry it)

use std::time::Duration;

use crate::cat::{CatCommand, CatDialect, CatResponse, CatSession};
use crate::domain::{Frequency, Psk31Error, Psk31Result, RadioStatus, RadioType};
use crate::ports::{RadioControl, SerialConnection};

/// Mode names from the shared table that the FT-891 does not have.
const UNSUPPORTED_MODES: &[&str] = &["DATA-FM", "C4FM"];

/// FT-891 radio adapter. Owns a CatSession and tracks TX state.
pub struct Ft891Radio {
    session: CatSession,
    is_transmitting: bool,
    /// Band-select code of the last frequency we sent, to avoid redundant
    /// `BS;` commands (each triggers a band-memory recall on Yaesu radios).
    last_band_code: Option<u8>,
}

impl Ft891Radio {
    pub fn new(serial: Box<dyn SerialConnection>) -> Self {
        Self {
            session: CatSession::with_dialect(serial, CatDialect::Ft891),
            is_transmitting: false,
            last_band_code: None,
        }
    }
}

impl RadioControl for Ft891Radio {
    fn ptt_on(&mut self) -> Psk31Result<()> {
        self.session.execute_write_only(&CatCommand::PttOn)?;
        self.is_transmitting = true;
        Ok(())
    }

    fn ptt_off(&mut self) -> Psk31Result<()> {
        self.session.execute_write_only(&CatCommand::PttOff)?;
        self.is_transmitting = false;
        Ok(())
    }

    fn is_transmitting(&self) -> bool {
        self.is_transmitting
    }

    fn get_frequency(&mut self) -> Psk31Result<Frequency> {
        match self.session.execute(&CatCommand::GetFrequencyA)? {
            // No band validation on read: the radio may be on a non-amateur frequency.
            CatResponse::FrequencyHz(hz) => Ok(Frequency::hz(hz as f64)),
            _ => Err(Psk31Error::Cat("unexpected response for GetFrequencyA".into())),
        }
    }

    fn set_frequency(&mut self, freq: Frequency) -> Psk31Result<()> {
        let hz = freq.as_hz() as u64;
        if !RadioType::Ft891.supports_frequency(hz) {
            return Err(Psk31Error::Cat(format!(
                "Frequency {hz} Hz is outside the FT-891's US amateur bands"
            )));
        }
        // 60m has no BS; code (None) — frequency only.
        if let Some(code) = RadioType::Ft891.band_select_code(hz) {
            if self.last_band_code != Some(code) {
                self.session.execute_write_only(&CatCommand::BandSelect(code))?;
                self.last_band_code = Some(code);
            }
        } else {
            // Next in-band frequency must re-send BS; after we've left the BS-coded bands.
            self.last_band_code = None;
        }
        self.session.execute_write_only(&CatCommand::SetFrequencyA(hz))?;
        Ok(())
    }

    fn get_mode(&mut self) -> Psk31Result<String> {
        match self.session.execute(&CatCommand::GetMode)? {
            CatResponse::Mode(name) => Ok(name),
            _ => Err(Psk31Error::Cat("unexpected response for GetMode".into())),
        }
    }

    fn set_mode(&mut self, mode: &str) -> Psk31Result<()> {
        if UNSUPPORTED_MODES.contains(&mode) {
            return Err(Psk31Error::Cat(format!("FT-891 does not support mode {mode}")));
        }
        self.session.execute_write_only(&CatCommand::SetMode(mode.to_string()))?;
        Ok(())
    }

    fn get_tx_power(&mut self) -> Psk31Result<u32> {
        match self.session.execute(&CatCommand::GetTxPower)? {
            CatResponse::TxPower(w) => Ok(w),
            _ => Err(Psk31Error::Cat("unexpected response for GetTxPower".into())),
        }
    }

    /// Values below the radio's 5 W minimum are raised to 5 W rather than rejected:
    /// callers on the TX path ignore a failed power set, which would leave the radio
    /// at whatever (possibly much higher) power it had before.
    fn set_tx_power(&mut self, watts: u32) -> Psk31Result<()> {
        let (min, max) = RadioType::Ft891.tx_power_range();
        if watts > max {
            return Err(Psk31Error::Cat(format!(
                "TX power {watts} W exceeds FT-891 maximum ({max} W)"
            )));
        }
        let watts = if watts < min {
            log::warn!("TX power {watts} W below FT-891 minimum; using {min} W");
            min
        } else {
            watts
        };
        self.session.execute_write_only(&CatCommand::SetTxPower(watts))?;
        Ok(())
    }

    fn get_signal_strength(&mut self) -> Psk31Result<f32> {
        match self.session.execute(&CatCommand::GetSignalStrength)? {
            CatResponse::SignalStrength(s) => Ok(s),
            _ => Err(Psk31Error::Cat("unexpected response for GetSignalStrength".into())),
        }
    }

    /// `IF;` for frequency/mode/RIT, then `TX;` for PTT state.
    fn get_status(&mut self) -> Psk31Result<RadioStatus> {
        let mut status = match self.session.execute(&CatCommand::GetStatus)? {
            CatResponse::Status(s) => s,
            _ => return Err(Psk31Error::Cat("unexpected response for GetStatus".into())),
        };
        status.is_transmitting = match self.session.execute(&CatCommand::GetTxState)? {
            CatResponse::TxState(tx) => tx,
            _ => return Err(Psk31Error::Cat("unexpected response for GetTxState".into())),
        };
        Ok(status)
    }
}

/// Safety: auto-release PTT if the radio is dropped while transmitting.
/// Retries up to 3 times with increasing delays in case the first attempt
/// fails (e.g. USB adapter momentarily busy).
impl Drop for Ft891Radio {
    fn drop(&mut self) {
        if self.is_transmitting {
            for delay_ms in [0, 10, 50] {
                if delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
                if self.session.execute_write_only(&CatCommand::PttOff).is_ok() {
                    self.is_transmitting = false;
                    return;
                }
            }
            eprintln!("CRITICAL: Failed to release PTT on drop. Radio may still be transmitting!");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Mock serial that replays queued replies, one per read-bearing command.
    struct MockSerial {
        log: Arc<Mutex<Vec<String>>>,
        replies: Arc<Mutex<Vec<String>>>,
        current: String,
    }

    impl SerialConnection for MockSerial {
        fn write(&mut self, data: &[u8]) -> Psk31Result<usize> {
            let wire = String::from_utf8_lossy(data).to_string();
            // Queries get the next queued reply; set commands never read.
            let mut q = self.replies.lock().unwrap();
            self.current = if q.is_empty() { ";".into() } else { q.remove(0) };
            self.log.lock().unwrap().push(wire);
            Ok(data.len())
        }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            let bytes = self.current.as_bytes();
            let n = bytes.len().min(buf.len());
            buf[..n].copy_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn close(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_connected(&self) -> bool { true }
    }

    fn make_radio(replies: &[&str]) -> (Ft891Radio, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mock = MockSerial {
            log: Arc::clone(&log),
            replies: Arc::new(Mutex::new(replies.iter().map(|s| s.to_string()).collect())),
            current: String::new(),
        };
        (Ft891Radio::new(Box::new(mock)), log)
    }

    fn sent(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    // --- PTT ---

    #[test]
    fn ptt_on_sends_tx1_and_tracks_state() {
        let (mut r, log) = make_radio(&[]);
        assert!(!r.is_transmitting());
        r.ptt_on().unwrap();
        assert_eq!(sent(&log), ["TX1;"]);
        assert!(r.is_transmitting());
        r.ptt_off().unwrap();
        assert_eq!(sent(&log), ["TX1;", "TX0;"]);
        assert!(!r.is_transmitting());
    }

    // --- Frequency ---

    #[test]
    fn set_frequency_sends_bs_then_fa() {
        let (mut r, log) = make_radio(&[]);
        r.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        assert_eq!(sent(&log), ["BS05;", "FA014070000;"]);
    }

    #[test]
    fn set_frequency_same_band_skips_bs() {
        let (mut r, log) = make_radio(&[]);
        r.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        r.set_frequency(Frequency::hz(14_074_000.0)).unwrap();
        assert_eq!(sent(&log), ["BS05;", "FA014070000;", "FA014074000;"]);
    }

    #[test]
    fn set_frequency_band_change_sends_bs() {
        let (mut r, log) = make_radio(&[]);
        r.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        r.set_frequency(Frequency::hz(7_035_000.0)).unwrap();
        assert_eq!(sent(&log), ["BS05;", "FA014070000;", "BS03;", "FA007035000;"]);
    }

    #[test]
    fn set_frequency_60m_sends_fa_only() {
        let (mut r, log) = make_radio(&[]);
        r.set_frequency(Frequency::hz(5_358_500.0)).unwrap();
        assert_eq!(sent(&log), ["FA005358500;"]);
    }

    #[test]
    fn returning_to_band_after_60m_resends_bs() {
        let (mut r, log) = make_radio(&[]);
        r.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        r.set_frequency(Frequency::hz(5_358_500.0)).unwrap();
        r.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        assert_eq!(
            sent(&log),
            ["BS05;", "FA014070000;", "FA005358500;", "BS05;", "FA014070000;"]
        );
    }

    #[test]
    fn set_frequency_rejects_unsupported_before_sending() {
        let (mut r, log) = make_radio(&[]);
        for hz in [144_200_000.0, 432_100_000.0, 11_000_000.0, 0.0] {
            assert!(r.set_frequency(Frequency::hz(hz)).is_err(), "{hz}");
        }
        assert!(sent(&log).is_empty(), "nothing should reach the wire");
    }

    #[test]
    fn get_frequency_sends_fa_query() {
        let (mut r, log) = make_radio(&["FA014070000;"]);
        assert_eq!(r.get_frequency().unwrap().as_hz(), 14_070_000.0);
        assert_eq!(sent(&log), ["FA;"]);
    }

    #[test]
    fn get_frequency_allows_non_amateur_frequency() {
        let (mut r, _) = make_radio(&["FA010000000;"]);
        assert_eq!(r.get_frequency().unwrap().as_hz(), 10_000_000.0);
    }

    // --- Mode ---

    #[test]
    fn set_mode_data_usb_sends_md0c() {
        let (mut r, log) = make_radio(&[]);
        r.set_mode("DATA-USB").unwrap();
        assert_eq!(sent(&log), ["MD0C;"]);
    }

    #[test]
    fn set_mode_rejects_modes_the_ft891_lacks() {
        let (mut r, log) = make_radio(&[]);
        assert!(r.set_mode("DATA-FM").is_err());
        assert!(r.set_mode("C4FM").is_err());
        assert!(sent(&log).is_empty());
    }

    #[test]
    fn get_mode_sends_md0_query() {
        let (mut r, log) = make_radio(&["MD08;"]);
        assert_eq!(r.get_mode().unwrap(), "DATA-LSB");
        assert_eq!(sent(&log), ["MD0;"]);
    }

    // --- TX power ---

    #[test]
    fn tx_power_get_and_set() {
        let (mut r, log) = make_radio(&["PC025;"]);
        assert_eq!(r.get_tx_power().unwrap(), 25);
        r.set_tx_power(10).unwrap();
        assert_eq!(sent(&log), ["PC;", "PC010;"]);
    }

    #[test]
    fn tx_power_boundaries() {
        let (mut r, log) = make_radio(&[]);
        r.set_tx_power(5).unwrap();
        r.set_tx_power(100).unwrap();
        assert_eq!(sent(&log), ["PC005;", "PC100;"]);
    }

    #[test]
    fn tx_power_below_minimum_is_raised_to_5w() {
        let (mut r, log) = make_radio(&[]);
        r.set_tx_power(0).unwrap();
        r.set_tx_power(4).unwrap();
        assert_eq!(sent(&log), ["PC005;", "PC005;"]);
    }

    #[test]
    fn tx_power_above_maximum_rejected_before_sending() {
        let (mut r, log) = make_radio(&[]);
        assert!(r.set_tx_power(101).is_err());
        assert!(sent(&log).is_empty());
    }

    // --- S-meter ---

    #[test]
    fn signal_strength_uses_3_digit_scale() {
        let (mut r, log) = make_radio(&["SM0255;", "SM0000;"]);
        assert_eq!(r.get_signal_strength().unwrap(), 1.0);
        assert_eq!(r.get_signal_strength().unwrap(), 0.0);
        assert_eq!(sent(&log), ["SM0;", "SM0;"]);
    }

    // --- Status ---

    /// 25-char body: ch 001, 14.070 MHz, clarifier +0000 off, mode 8 (DATA-LSB), no CTCSS/shift.
    const IF_RX: &str = "IF001014070000+000000800000;";

    #[test]
    fn get_status_queries_if_then_tx() {
        let (mut r, log) = make_radio(&[IF_RX, "TX0;"]);
        let s = r.get_status().unwrap();
        assert_eq!(sent(&log), ["IF;", "TX;"]);
        assert_eq!(s.frequency_hz, 14_070_000);
        assert_eq!(s.mode, "DATA-LSB");
        assert!(!s.is_transmitting);
    }

    #[test]
    fn get_status_tx_flag_comes_from_tx_query() {
        for tx in ["TX1;", "TX2;"] {
            let (mut r, _) = make_radio(&[IF_RX, tx]);
            assert!(r.get_status().unwrap().is_transmitting, "{tx}");
        }
    }

    // --- Error paths ---

    #[test]
    fn unexpected_replies_are_errors() {
        let (mut r, _) = make_radio(&["XX;"]);
        assert!(r.get_frequency().is_err());
        let (mut r, _) = make_radio(&["XX;"]);
        assert!(r.get_mode().is_err());
        let (mut r, _) = make_radio(&["XX;"]);
        assert!(r.get_tx_power().is_err());
        let (mut r, _) = make_radio(&["XX;"]);
        assert!(r.get_signal_strength().is_err());
        let (mut r, _) = make_radio(&["XX;"]);
        assert!(r.get_status().is_err());
        let (mut r, _) = make_radio(&[IF_RX, "XX;"]);
        assert!(r.get_status().is_err(), "bad TX reply");
    }

    #[test]
    fn nak_is_error() {
        let (mut r, _) = make_radio(&["?;"]);
        assert!(r.get_frequency().is_err());
    }

    // --- Drop: PTT safety ---

    #[test]
    fn drop_while_transmitting_sends_ptt_off() {
        let (mut r, log) = make_radio(&[]);
        r.ptt_on().unwrap();
        drop(r);
        assert_eq!(sent(&log), ["TX1;", "TX0;"]);
    }

    #[test]
    fn drop_while_idle_sends_nothing() {
        let (r, log) = make_radio(&[]);
        drop(r);
        assert!(sent(&log).is_empty());
    }
}
