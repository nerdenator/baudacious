//! FT-991A radio adapter using CAT (Computer Aided Transceiver) commands
//!
//! The FT-991A speaks a simple text protocol over serial:
//! - Send a command like `FA;` (get frequency)
//! - Radio replies like `FA00014070000;` (frequency in Hz, zero-padded to 11 digits)
//! - All commands/responses are terminated with `;`
//!
//! This adapter translates high-level RadioControl calls into CatCommands,
//! delegates I/O to CatSession, and interprets CatResponses back into domain types.

use std::time::Duration;

use crate::cat::{CatCommand, CatResponse, CatSession};
use crate::domain::{Frequency, Psk31Error, Psk31Result, RadioStatus};
use crate::ports::{RadioControl, SerialConnection};

/// US amateur radio bands (FCC Part 97) as (low_hz, high_hz) pairs.
/// Only frequencies within these bands are accepted.
const AMATEUR_BANDS_HZ: &[(u64, u64)] = &[
    (1_800_000, 2_000_000),       // 160m
    (3_500_000, 4_000_000),       // 80m
    (5_332_000, 5_405_000),       // 60m
    (7_000_000, 7_300_000),       // 40m
    (10_100_000, 10_150_000),     // 30m
    (14_000_000, 14_350_000),     // 20m
    (18_068_000, 18_168_000),     // 17m
    (21_000_000, 21_450_000),     // 15m
    (24_890_000, 24_990_000),     // 12m
    (28_000_000, 29_700_000),     // 10m
    (50_000_000, 54_000_000),     // 6m
    (144_000_000, 148_000_000),   // 2m
    (420_000_000, 450_000_000),   // 70cm
];

/// Check if a frequency falls within a US amateur band.
fn is_amateur_frequency(hz: u64) -> bool {
    AMATEUR_BANDS_HZ.iter().any(|&(lo, hi)| hz >= lo && hz <= hi)
}

/// FT-991A radio adapter. Owns a CatSession and tracks TX state.
pub struct Ft991aRadio {
    session: CatSession,
    is_transmitting: bool,
}

impl Ft991aRadio {
    pub fn new(serial: Box<dyn SerialConnection>) -> Self {
        Self {
            session: CatSession::new(serial),
            is_transmitting: false,
        }
    }
}

impl RadioControl for Ft991aRadio {
    fn ptt_on(&mut self) -> Psk31Result<()> {
        self.session.execute(&CatCommand::PttOn)?;
        self.is_transmitting = true;
        Ok(())
    }

    fn ptt_off(&mut self) -> Psk31Result<()> {
        self.session.execute(&CatCommand::PttOff)?;
        self.is_transmitting = false;
        Ok(())
    }

    fn is_transmitting(&self) -> bool {
        self.is_transmitting
    }

    fn get_frequency(&mut self) -> Psk31Result<Frequency> {
        match self.session.execute(&CatCommand::GetFrequencyA)? {
            CatResponse::FrequencyHz(hz) => {
                if !is_amateur_frequency(hz) {
                    return Err(Psk31Error::Cat(format!(
                        "Frequency {hz} Hz is outside US amateur bands"
                    )));
                }
                Ok(Frequency::hz(hz as f64))
            }
            _ => Err(Psk31Error::Cat(
                "unexpected response for GetFrequencyA".into(),
            )),
        }
    }

    fn set_frequency(&mut self, freq: Frequency) -> Psk31Result<()> {
        let hz = freq.as_hz() as u64;
        if !is_amateur_frequency(hz) {
            return Err(Psk31Error::Cat(format!(
                "Frequency {hz} Hz is outside US amateur bands"
            )));
        }
        self.session.execute(&CatCommand::SetFrequencyA(hz))?;
        Ok(())
    }

    fn get_mode(&mut self) -> Psk31Result<String> {
        match self.session.execute(&CatCommand::GetMode)? {
            CatResponse::Mode(name) => Ok(name),
            _ => Err(Psk31Error::Cat("unexpected response for GetMode".into())),
        }
    }

    fn set_mode(&mut self, mode: &str) -> Psk31Result<()> {
        self.session.execute(&CatCommand::SetMode(mode.to_string()))?;
        Ok(())
    }

    fn get_tx_power(&mut self) -> Psk31Result<u32> {
        match self.session.execute(&CatCommand::GetTxPower)? {
            CatResponse::TxPower(w) => Ok(w),
            _ => Err(Psk31Error::Cat("unexpected response for GetTxPower".into())),
        }
    }

    fn set_tx_power(&mut self, watts: u32) -> Psk31Result<()> {
        if watts > 100 {
            return Err(Psk31Error::Cat(format!(
                "TX power {watts} W exceeds FT-991A maximum (100 W)"
            )));
        }
        self.session.execute(&CatCommand::SetTxPower(watts))?;
        Ok(())
    }

    fn get_signal_strength(&mut self) -> Psk31Result<f32> {
        match self.session.execute(&CatCommand::GetSignalStrength)? {
            CatResponse::SignalStrength(s) => Ok(s),
            _ => Err(Psk31Error::Cat(
                "unexpected response for GetSignalStrength".into(),
            )),
        }
    }

    fn get_status(&mut self) -> Psk31Result<RadioStatus> {
        match self.session.execute(&CatCommand::GetStatus)? {
            CatResponse::Status(s) => {
                if !is_amateur_frequency(s.frequency_hz) {
                    return Err(Psk31Error::Cat(format!(
                        "Frequency {} Hz is outside US amateur bands",
                        s.frequency_hz
                    )));
                }
                Ok(s)
            }
            _ => Err(Psk31Error::Cat(
                "unexpected response for GetStatus".into(),
            )),
        }
    }
}

/// Safety: auto-release PTT if the radio is dropped while transmitting.
/// Retries up to 3 times with increasing delays in case the first attempt
/// fails (e.g. USB adapter momentarily busy).
impl Drop for Ft991aRadio {
    fn drop(&mut self) {
        if self.is_transmitting {
            for delay_ms in [0, 10, 50] {
                if delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
                if self.session.execute(&CatCommand::PttOff).is_ok() {
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
    use crate::ports::SerialConnection;
    use std::sync::{Arc, Mutex};

    // ---------------------------------------------------------------------------
    // MockSerial for Ft991aRadio integration tests.
    // Wire-format and parse tests live in cat/encode.rs and cat/decode.rs.
    // These tests cover Ft991aRadio-level behaviour: validation, response routing.
    // ---------------------------------------------------------------------------

    struct MockSerial {
        log: Arc<Mutex<Vec<String>>>,
        response: String,
    }

    impl SerialConnection for MockSerial {
        fn write(&mut self, data: &[u8]) -> Psk31Result<usize> {
            self.log
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(data).into());
            Ok(data.len())
        }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            let bytes = self.response.as_bytes();
            let n = bytes.len().min(buf.len());
            buf[..n].copy_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn close(&mut self) -> Psk31Result<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
    }

    fn make_radio(response: &str) -> (Ft991aRadio, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mock = MockSerial {
            log: Arc::clone(&log),
            response: response.to_string(),
        };
        (Ft991aRadio::new(Box::new(mock)), log)
    }

    // --- Command wire strings (integration smoke tests) ---

    #[test]
    fn ptt_on_sends_tx1() {
        let (mut radio, log) = make_radio(";");
        radio.ptt_on().unwrap();
        assert_eq!(log.lock().unwrap()[0], "TX1;");
    }

    #[test]
    fn ptt_off_sends_tx0() {
        let (mut radio, log) = make_radio(";");
        radio.is_transmitting = true;
        radio.ptt_off().unwrap();
        assert_eq!(log.lock().unwrap()[0], "TX0;");
    }

    #[test]
    fn set_frequency_sends_correct_cat() {
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        assert_eq!(log.lock().unwrap()[0], "FA00014070000;");
    }

    #[test]
    fn set_frequency_40m_psk31() {
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(7_035_000.0)).unwrap();
        assert_eq!(log.lock().unwrap()[0], "FA00007035000;");
    }

    #[test]
    fn get_frequency_sends_fa_query() {
        let (mut radio, log) = make_radio("FA00014070000;");
        radio.get_frequency().unwrap();
        assert_eq!(log.lock().unwrap()[0], "FA;");
    }

    #[test]
    fn set_mode_data_usb_sends_md0c() {
        let (mut radio, log) = make_radio(";");
        radio.set_mode("DATA-USB").unwrap();
        assert_eq!(log.lock().unwrap()[0], "MD0C;");
    }

    #[test]
    fn get_mode_sends_md0_query() {
        let (mut radio, log) = make_radio("MD0C;");
        radio.get_mode().unwrap();
        assert_eq!(log.lock().unwrap()[0], "MD0;");
    }

    // --- Amateur band validation (Ft991aRadio responsibility) ---

    #[test]
    fn set_frequency_rejects_non_amateur_before_sending() {
        let (mut radio, log) = make_radio(";");
        // 10 MHz is between 30m and 20m bands — not an amateur allocation
        let result = radio.set_frequency(Frequency::hz(10_000_000.0));
        assert!(result.is_err(), "expected error for out-of-band frequency");
        assert!(
            log.lock().unwrap().is_empty(),
            "no bytes should reach the wire for out-of-band frequency"
        );
    }

    #[test]
    fn get_frequency_rejects_non_amateur_response() {
        // Radio returns a broadcast frequency — we should reject it
        let (mut radio, _) = make_radio("FA00001000000;");
        assert!(radio.get_frequency().is_err());
    }

    // --- TX power ---

    #[test]
    fn get_tx_power_sends_pc_query() {
        let (mut radio, log) = make_radio("PC025;");
        let watts = radio.get_tx_power().unwrap();
        assert_eq!(watts, 25);
        assert_eq!(log.lock().unwrap()[0], "PC;");
    }

    #[test]
    fn set_tx_power_sends_pc_wire() {
        let (mut radio, log) = make_radio(";");
        radio.set_tx_power(50).unwrap();
        assert_eq!(log.lock().unwrap()[0], "PC050;");
    }

    // --- S-meter ---

    #[test]
    fn get_signal_strength_sends_sm0_query() {
        let (mut radio, log) = make_radio("SM00015;");
        let level = radio.get_signal_strength().unwrap();
        assert_eq!(log.lock().unwrap()[0], "SM0;");
        assert_eq!(level, 0.5); // 15/30
    }

    // --- Status (IF;) ---

    fn make_if_body(freq: u64, mode_code: &str, tx: bool, rit_en: bool, rit_offset: i32, split: bool) -> String {
        let freq_str = format!("{freq:011}");
        let mode_padded = format!("0{mode_code}");
        let rit_sign = if rit_offset < 0 { '-' } else { '+' };
        let rit_abs = rit_offset.unsigned_abs();
        let rit_str = format!("{rit_sign}{rit_abs:04}");
        let rit_on = if rit_en { '1' } else { '0' };
        let tx_char = if tx { '1' } else { '0' };
        let split_char = if split { '1' } else { '0' };
        format!("IF{freq_str}     {rit_str}{rit_on}0  0{tx_char}{mode_padded}00{split_char}00000;")
    }

    #[test]
    fn get_status_sends_if_query() {
        let response = make_if_body(14_070_000, "C", false, false, 0, false);
        let (mut radio, log) = make_radio(&response);
        let status = radio.get_status().unwrap();
        assert_eq!(log.lock().unwrap()[0], "IF;");
        assert_eq!(status.frequency_hz, 14_070_000);
        assert_eq!(status.mode, "DATA-USB");
    }

    #[test]
    fn get_status_rejects_non_amateur_frequency() {
        // 10 MHz is between 30m and 20m, not an amateur allocation
        let response = make_if_body(10_000_000, "C", false, false, 0, false);
        let (mut radio, _) = make_radio(&response);
        assert!(radio.get_status().is_err());
    }

    #[test]
    fn set_tx_power_rejects_over_100w() {
        // Regression: >100 W produces a 4-digit PC wire string the radio rejects.
        // The validation must fire before any bytes reach the wire.
        let (mut radio, log) = make_radio(";");
        assert!(radio.set_tx_power(101).is_err());
        assert!(
            log.lock().unwrap().is_empty(),
            "no bytes should reach the wire for out-of-range power"
        );
    }
}
