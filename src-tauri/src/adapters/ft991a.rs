//! FT-991A radio adapter using CAT (Computer Aided Transceiver) commands
//!
//! The FT-991A speaks a simple text protocol over serial:
//! - Send a command like `FA;` (get frequency)
//! - Radio replies like `FA014070000;` (frequency in Hz, 9-digit zero-padded)
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

/// Map a frequency to the FT-991A BS; band-select code.
///
/// Codes: 0=160m, 1=80m, 2=60m, 3=40m, 4=30m, 5=20m, 6=17m, 7=15m, 8=12m,
///        9=10m, 10=6m, 12=2m, 13=70cm.
fn band_select_code(hz: u64) -> u8 {
    match hz {
        1_800_000..=2_000_000 => 0,
        3_500_000..=4_000_000 => 1,
        5_332_000..=5_405_000 => 2,
        7_000_000..=7_300_000 => 3,
        10_100_000..=10_150_000 => 4,
        14_000_000..=14_350_000 => 5,
        18_068_000..=18_168_000 => 6,
        21_000_000..=21_450_000 => 7,
        24_890_000..=24_990_000 => 8,
        28_000_000..=29_700_000 => 9,
        50_000_000..=54_000_000 => 10,
        144_000_000..=148_000_000 => 12,
        420_000_000..=450_000_000 => 13,
        _ => 5,
    }
}

/// FT-991A radio adapter. Owns a CatSession and tracks TX state.
pub struct Ft991aRadio {
    session: CatSession,
    is_transmitting: bool,
    /// Band-select code of the last frequency we sent, used to avoid
    /// redundant BS; commands (which trigger a full band-memory recall
    /// and reset DSP settings like filter width and noise reduction).
    last_band_code: Option<u8>,
}

impl Ft991aRadio {
    pub fn new(serial: Box<dyn SerialConnection>) -> Self {
        Self {
            session: CatSession::new(serial),
            is_transmitting: false,
            last_band_code: None,
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
                // No band validation here — reading the radio's current frequency
                // should always succeed (radio might be on a non-amateur freq).
                // Validation only applies in set_frequency.
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
        let code = band_select_code(hz);
        // Only send BS; when the band actually changes. BS; triggers a full
        // band-memory recall on the FT-991A (filters, noise reduction, etc.),
        // so sending it on every within-band frequency change is disruptive.
        if self.last_band_code != Some(code) {
            self.session.execute_write_only(&CatCommand::BandSelect(code))?;
            self.last_band_code = Some(code);
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
        self.session.execute_write_only(&CatCommand::SetMode(mode.to_string()))?;
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
            CatResponse::Status(s) => Ok(s),
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
    fn is_transmitting_reflects_ptt_state() {
        let (mut radio, _) = make_radio(";");
        assert!(!radio.is_transmitting(), "should start not transmitting");
        radio.is_transmitting = true;
        assert!(radio.is_transmitting(), "should report transmitting after ptt_on");
        radio.is_transmitting = false;
        assert!(!radio.is_transmitting(), "should report not transmitting after ptt_off");
    }

    #[test]
    fn set_frequency_sends_bs_then_fa() {
        // Both BS; and FA; are write-only (no ack). Mock response is irrelevant.
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(14_070_000.0)).unwrap();
        let cmds = log.lock().unwrap();
        assert_eq!(cmds[0], "BS05;");
        assert_eq!(cmds[1], "FA014070000;");
    }

    #[test]
    fn set_frequency_same_band_skips_bs() {
        // Second set_frequency within the same band should only send FA;, not BS;.
        // BS; triggers a full band-memory recall (filters, NR, etc.) and must be
        // suppressed when only the frequency changes within the same band.
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(14_070_000.0)).unwrap(); // 20m → sends BS05 + FA
        radio.set_frequency(Frequency::hz(14_074_000.0)).unwrap(); // 20m again → FA only
        let cmds = log.lock().unwrap();
        assert_eq!(cmds[0], "BS05;");
        assert_eq!(cmds[1], "FA014070000;");
        assert_eq!(cmds[2], "FA014074000;"); // no BS; here
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn set_frequency_band_change_sends_bs() {
        // Moving to a different band must still send BS; to switch VFO stack.
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(14_070_000.0)).unwrap(); // 20m
        radio.set_frequency(Frequency::hz(7_035_000.0)).unwrap();  // 40m → new BS;
        let cmds = log.lock().unwrap();
        assert_eq!(cmds[0], "BS05;"); // 20m
        assert_eq!(cmds[1], "FA014070000;");
        assert_eq!(cmds[2], "BS03;"); // 40m
        assert_eq!(cmds[3], "FA007035000;");
    }

    #[test]
    fn set_frequency_40m() {
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(7_035_000.0)).unwrap();
        let cmds = log.lock().unwrap();
        assert_eq!(cmds[0], "BS03;");
        assert_eq!(cmds[1], "FA007035000;");
    }

    #[test]
    fn set_frequency_17m() {
        let (mut radio, log) = make_radio(";");
        radio.set_frequency(Frequency::hz(18_100_000.0)).unwrap();
        let cmds = log.lock().unwrap();
        assert_eq!(cmds[0], "BS06;");
        assert_eq!(cmds[1], "FA018100000;");
    }

    #[test]
    fn get_frequency_sends_fa_query() {
        let (mut radio, log) = make_radio("FA014070000;");
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
    fn get_frequency_returns_non_amateur_response() {
        // get_frequency is a read — it should succeed even for non-amateur frequencies.
        // (Band validation only applies to set_frequency.)
        let (mut radio, _) = make_radio("FA001000000;");
        assert!(radio.get_frequency().is_ok());
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
    fn get_status_allows_non_amateur_frequency() {
        // Connect should succeed regardless of current VFO frequency —
        // band validation only applies to set_frequency, not status reads.
        let response = make_if_body(10_000_000, "C", false, false, 0, false);
        let (mut radio, _) = make_radio(&response);
        assert!(radio.get_status().is_ok());
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

    // --- is_amateur_frequency: exact band edges ---

    #[test]
    fn is_amateur_frequency_exact_band_edges() {
        // Each band: lower edge, upper edge, one below, one above
        let cases: &[(u64, bool)] = &[
            (1_800_000, true),   // 160m lower
            (2_000_000, true),   // 160m upper
            (1_799_999, false),  // just below 160m
            (2_000_001, false),  // just above 160m
            (3_500_000, true),   // 80m lower
            (4_000_000, true),   // 80m upper
            (3_499_999, false),
            (4_000_001, false),
            (5_332_000, true),   // 60m lower
            (5_405_000, true),   // 60m upper
            (5_331_999, false),
            (5_405_001, false),
            (7_000_000, true),   // 40m lower
            (7_300_000, true),   // 40m upper
            (10_100_000, true),  // 30m lower
            (10_150_000, true),  // 30m upper
            (14_000_000, true),  // 20m lower
            (14_350_000, true),  // 20m upper
            (18_068_000, true),  // 17m lower
            (18_168_000, true),  // 17m upper
            (21_000_000, true),  // 15m lower
            (21_450_000, true),  // 15m upper
            (24_890_000, true),  // 12m lower
            (24_990_000, true),  // 12m upper
            (28_000_000, true),  // 10m lower
            (29_700_000, true),  // 10m upper
            (50_000_000, true),  // 6m lower
            (54_000_000, true),  // 6m upper
            (144_000_000, true), // 2m lower
            (148_000_000, true), // 2m upper
            (420_000_000, true), // 70cm lower
            (450_000_000, true), // 70cm upper
            (450_000_001, false),// above 70cm
            (10_000_000, false), // gap between 30m and 20m
            (0, false),          // DC
        ];
        for &(hz, expected) in cases {
            assert_eq!(
                is_amateur_frequency(hz),
                expected,
                "is_amateur_frequency({hz}) should be {expected}"
            );
        }
    }

    // --- band_select_code: every band arm and the fallback ---

    #[test]
    fn band_select_code_all_bands() {
        let cases: &[(u64, u8)] = &[
            (1_800_000, 0),     // 160m
            (3_500_000, 1),     // 80m
            (5_332_000, 2),     // 60m
            (7_000_000, 3),     // 40m
            (10_100_000, 4),    // 30m
            (14_000_000, 5),    // 20m
            (18_068_000, 6),    // 17m
            (21_000_000, 7),    // 15m
            (24_890_000, 8),    // 12m
            (28_000_000, 9),    // 10m
            (50_000_000, 10),   // 6m
            (144_000_000, 12),  // 2m
            (420_000_000, 13),  // 70cm
            (10_000_000, 5),    // gap → fallback to 20m (code 5)
        ];
        for &(hz, expected) in cases {
            assert_eq!(
                band_select_code(hz),
                expected,
                "band_select_code({hz}) should be {expected}"
            );
        }
    }

    // --- Drop impl: PTT safety ---

    #[test]
    fn drop_while_transmitting_sends_ptt_off() {
        // Verify the Drop impl sends TX0; when dropped while is_transmitting is true.
        let log = Arc::new(Mutex::new(Vec::new()));
        let mock = MockSerial {
            log: Arc::clone(&log),
            response: ";".to_string(),
        };
        let mut radio = Ft991aRadio::new(Box::new(mock));
        radio.is_transmitting = true;
        drop(radio);
        let cmds = log.lock().unwrap();
        assert!(
            cmds.iter().any(|c| c == "TX0;"),
            "Drop should send TX0; when is_transmitting is true, got: {:?}",
            *cmds
        );
    }

    #[test]
    fn drop_while_not_transmitting_sends_no_ptt_off() {
        // When is_transmitting is false, Drop should not send any commands.
        let log = Arc::new(Mutex::new(Vec::new()));
        let mock = MockSerial {
            log: Arc::clone(&log),
            response: ";".to_string(),
        };
        let radio = Ft991aRadio::new(Box::new(mock));
        // is_transmitting defaults to false
        drop(radio);
        assert!(
            log.lock().unwrap().is_empty(),
            "Drop should not send commands when is_transmitting is false"
        );
    }

    // --- is_amateur_frequency: VHF exact lower boundaries and between-band gaps ---

    #[test]
    fn is_amateur_frequency_vhf_lower_boundaries() {
        // Exact lower boundary of each VHF/UHF band
        assert!(is_amateur_frequency(50_000_000), "50 MHz (6m lower) should be amateur");
        assert!(is_amateur_frequency(144_000_000), "144 MHz (2m lower) should be amateur");
        assert!(is_amateur_frequency(420_000_000), "420 MHz (70cm lower) should be amateur");
        // Just below each VHF/UHF band
        assert!(!is_amateur_frequency(49_999_999), "49.999999 MHz should not be amateur");
        assert!(!is_amateur_frequency(143_999_999), "143.999999 MHz should not be amateur");
        assert!(!is_amateur_frequency(419_999_999), "419.999999 MHz should not be amateur");
    }

    #[test]
    fn is_amateur_frequency_between_hf_bands() {
        // Between 30m (10.1–10.15 MHz) and 20m (14.0–14.35 MHz)
        assert!(!is_amateur_frequency(11_000_000), "11 MHz (between 30m and 20m) should not be amateur");
        // Between 17m (18.068–18.168) and 15m (21.0–21.45 MHz)
        assert!(!is_amateur_frequency(19_000_000), "19 MHz (between 17m and 15m) should not be amateur");
    }

    // --- band_select_code: fallback arm ---

    #[test]
    fn band_select_code_between_bands_returns_fallback() {
        // 11 MHz is between 30m and 20m — hits the `_ => 5` fallback
        assert_eq!(band_select_code(11_000_000), 5, "out-of-band frequency should return fallback code 5");
    }

    // --- get_status: full RadioStatus field assertions ---

    #[test]
    fn get_status_tx_flag_and_split() {
        let response = make_if_body(14_070_000, "C", true, false, 0, true);
        let (mut radio, _) = make_radio(&response);
        let status = radio.get_status().unwrap();
        assert_eq!(status.frequency_hz, 14_070_000);
        assert_eq!(status.mode, "DATA-USB");
        assert!(status.is_transmitting, "is_transmitting should be true");
        assert!(status.split, "split should be true");
    }

    #[test]
    fn get_status_rit_enabled_and_offset() {
        let response = make_if_body(7_035_000, "1", false, true, -200, false);
        let (mut radio, _) = make_radio(&response);
        let status = radio.get_status().unwrap();
        assert_eq!(status.frequency_hz, 7_035_000);
        assert_eq!(status.mode, "LSB");
        assert!(status.rit_enabled, "rit_enabled should be true");
        assert_eq!(status.rit_offset_hz, -200, "rit_offset_hz should be -200");
    }

    // --- get_signal_strength: value in 0.0..=1.0 ---

    #[test]
    fn get_signal_strength_returns_value_in_range() {
        // SM00015; → 15/30 = 0.5 (already tested in existing test, but also verify bounds)
        let (mut radio, _) = make_radio("SM00015;");
        let level = radio.get_signal_strength().unwrap();
        assert!((0.0..=1.0).contains(&level), "signal strength should be in 0.0..=1.0, got {level}");
    }

    #[test]
    fn get_signal_strength_max_value() {
        // SM00030; → 30/30 = 1.0
        let (mut radio, _) = make_radio("SM00030;");
        let level = radio.get_signal_strength().unwrap();
        assert!((0.0..=1.0).contains(&level), "signal strength at max should be in 0.0..=1.0, got {level}");
    }

    #[test]
    fn get_signal_strength_min_value() {
        // SM00000; → 0/30 = 0.0
        let (mut radio, _) = make_radio("SM00000;");
        let level = radio.get_signal_strength().unwrap();
        assert_eq!(level, 0.0, "signal strength at zero should be 0.0");
    }

    // --- Unexpected response type → Err paths ---

    #[test]
    fn get_frequency_unexpected_response_is_err() {
        // Returning ";" gives CatResponse::Ack, not FrequencyHz → triggers _ => Err
        let (mut radio, _) = make_radio(";");
        assert!(radio.get_frequency().is_err());
    }

    #[test]
    fn get_mode_unexpected_response_is_err() {
        let (mut radio, _) = make_radio(";");
        assert!(radio.get_mode().is_err());
    }

    #[test]
    fn get_tx_power_unexpected_response_is_err() {
        let (mut radio, _) = make_radio(";");
        assert!(radio.get_tx_power().is_err());
    }

    #[test]
    fn get_signal_strength_unexpected_response_is_err() {
        let (mut radio, _) = make_radio(";");
        assert!(radio.get_signal_strength().is_err());
    }

    #[test]
    fn get_status_unexpected_response_is_err() {
        let (mut radio, _) = make_radio(";");
        assert!(radio.get_status().is_err());
    }

    // --- set_frequency covers every band via BS; code ---

    #[test]
    fn set_frequency_all_bands_send_correct_bs_code() {
        // Sample one frequency from every band and verify the BS; wire prefix
        let cases: &[(f64, &str)] = &[
            (1_900_000.0, "BS00;"),
            (3_600_000.0, "BS01;"),
            (5_358_500.0, "BS02;"),
            (7_035_000.0, "BS03;"),
            (10_120_000.0, "BS04;"),
            (14_070_000.0, "BS05;"),
            (18_100_000.0, "BS06;"),
            (21_070_000.0, "BS07;"),
            (24_920_000.0, "BS08;"),
            (28_120_000.0, "BS09;"),
            (50_313_000.0, "BS10;"),
            (144_200_000.0, "BS12;"),
            (432_100_000.0, "BS13;"),
        ];
        for &(hz, expected_bs) in cases {
            let (mut radio, log) = make_radio(";");
            radio.set_frequency(Frequency::hz(hz)).unwrap();
            let cmds = log.lock().unwrap();
            assert_eq!(
                cmds[0], expected_bs,
                "set_frequency({hz}) should send {expected_bs}"
            );
        }
    }
}
