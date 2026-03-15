//! Phase 8 integration tests: command-layer radio control
//!
//! These tests verify the full path from AppState → RadioControl adapter
//! without a Tauri runtime or real hardware. Two adapters are exercised:
//!
//! - `MockRadio` — in-memory state, used for connect/disconnect/PTT/mode tests
//! - `Ft991aRadio` + local `MockSerial` — used for CAT wire-format and
//!   band-validation tests (the layer that matters for real hardware safety)
//!
//! Run with: cargo test --manifest-path src-tauri/Cargo.toml

use std::sync::{Arc, Mutex};

use baudacious_lib::adapters::ft991a::Ft991aRadio;
use baudacious_lib::adapters::mock_radio::MockRadio;
use baudacious_lib::domain::data_mode_for_frequency;
use baudacious_lib::domain::{Frequency, Psk31Result};
use baudacious_lib::ports::{RadioControl, SerialConnection};
use baudacious_lib::state::AppState;

// ---------------------------------------------------------------------------
// Local MockSerial — gives Ft991aRadio a fake serial port.
// Captures every byte written; returns a single preconfigured response.
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

fn make_ft991a(response: &str) -> (Ft991aRadio, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mock = MockSerial {
        log: Arc::clone(&log),
        response: response.to_string(),
    };
    (Ft991aRadio::new(Box::new(mock)), log)
}

/// Insert a MockRadio into AppState, simulating a successful connect.
fn connected_state() -> AppState {
    let state = AppState::new();
    *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
    *state.serial_port_name.lock().unwrap() = Some("mock".to_string());
    state
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// After simulated connect, AppState.radio is Some; after disconnect, None.
#[test]
fn connect_stores_radio_in_state_and_disconnect_releases_it() {
    let state = AppState::new();
    assert!(state.radio.lock().unwrap().is_none());
    assert!(state.serial_port_name.lock().unwrap().is_none());

    // Simulate connect
    *state.radio.lock().unwrap() = Some(Box::new(MockRadio::new()));
    *state.serial_port_name.lock().unwrap() = Some("mock".to_string());

    assert!(state.radio.lock().unwrap().is_some());
    assert_eq!(
        state.serial_port_name.lock().unwrap().as_deref(),
        Some("mock")
    );

    // Simulate disconnect
    *state.radio.lock().unwrap() = None;
    *state.serial_port_name.lock().unwrap() = None;

    assert!(state.radio.lock().unwrap().is_none());
    assert!(state.serial_port_name.lock().unwrap().is_none());
}

/// MockRadio initialises with 20m PSK-31 calling frequency and DATA-USB mode.
#[test]
fn connect_returns_correct_radio_info() {
    let mut radio = MockRadio::new();
    assert_eq!(radio.get_frequency().unwrap().as_hz(), 14_070_000.0);
    assert_eq!(radio.get_mode().unwrap(), "DATA-USB");
}

/// set_frequency persists so a subsequent get_frequency reflects the change.
#[test]
fn set_frequency_updates_mock_state() {
    let mut radio = MockRadio::new();
    radio.set_frequency(Frequency::hz(7_035_000.0)).unwrap();
    assert_eq!(radio.get_frequency().unwrap().as_hz(), 7_035_000.0);
}

/// set_frequency on Ft991aRadio rejects an out-of-band frequency before any
/// bytes reach the wire — the key safety guarantee from Phase 8.
#[test]
fn set_frequency_rejects_non_amateur_before_sending() {
    let (mut radio, log) = make_ft991a(";");
    // 10 MHz sits between 30m and 20m — not a US amateur allocation
    let result = radio.set_frequency(Frequency::hz(10_000_000.0));
    assert!(result.is_err(), "expected Err for out-of-band frequency");
    assert!(
        log.lock().unwrap().is_empty(),
        "no bytes should hit the wire for an out-of-band frequency"
    );
}

/// Amateur band frequencies go through without error on Ft991aRadio.
#[test]
fn set_frequency_accepts_all_amateur_bands() {
    let freqs: &[(f64, &str)] = &[
        (1_838_000.0, "160m"),
        (3_580_000.0, "80m"),
        (7_035_000.0, "40m"),
        (10_142_000.0, "30m"),
        (14_070_000.0, "20m"),
        (18_100_000.0, "17m"),
        (21_080_000.0, "15m"),
        (24_920_000.0, "12m"),
        (28_120_000.0, "10m"),
    ];
    for &(hz, band) in freqs {
        let (mut radio, _) = make_ft991a(";");
        assert!(
            radio.set_frequency(Frequency::hz(hz)).is_ok(),
            "expected Ok for {band} ({hz} Hz)"
        );
    }
}

/// PTT on → is_transmitting true; PTT off → is_transmitting false.
#[test]
fn ptt_on_off_round_trip() {
    let mut radio = MockRadio::new();
    assert!(!radio.is_transmitting());
    radio.ptt_on().unwrap();
    assert!(radio.is_transmitting());
    radio.ptt_off().unwrap();
    assert!(!radio.is_transmitting());
}

/// get_mode after set_mode returns the new mode.
#[test]
fn set_mode_round_trip() {
    let mut radio = MockRadio::new();
    for mode in ["USB", "LSB", "DATA-USB", "CW"] {
        radio.set_mode(mode).unwrap();
        assert_eq!(radio.get_mode().unwrap(), mode);
    }
}

/// After disconnect, AppState.radio is None and radio commands cannot proceed.
#[test]
fn radio_commands_fail_when_not_connected() {
    let state = AppState::new();
    // No radio connected — lock returns None
    let mut lock = state.radio.lock().unwrap();
    let result: Result<(), &str> = match lock.as_mut() {
        Some(_) => Ok(()),
        None => Err("no radio connected"),
    };
    assert!(result.is_err());
}

/// Selecting 40m band sends BS03; (band select) then FA; in 9-digit format.
#[test]
fn band_change_40m_sends_psk31_calling_frequency() {
    let (mut radio, log) = make_ft991a(";");
    // This matches BAND_PLAN[40m].psk31Hz in serial-panel.ts
    radio.set_frequency(Frequency::hz(7_035_000.0)).unwrap();
    let cmds = log.lock().unwrap();
    assert_eq!(cmds[0], "BS03;");
    assert_eq!(cmds[1], "FA007035000;");
}

// ---------------------------------------------------------------------------
// A3: Data mode guard — data_mode_for_frequency + ensure_data_mode behaviour
// ---------------------------------------------------------------------------

/// Pure helper: frequencies below 10 MHz map to DATA-LSB.
#[test]
fn data_mode_for_low_hf_bands() {
    assert_eq!(data_mode_for_frequency(1_838_000.0), "DATA-LSB"); // 160m
    assert_eq!(data_mode_for_frequency(3_580_000.0), "DATA-LSB"); // 80m
    assert_eq!(data_mode_for_frequency(7_035_000.0), "DATA-LSB"); // 40m
    assert_eq!(data_mode_for_frequency(9_999_999.0), "DATA-LSB"); // just below threshold
}

/// Pure helper: frequencies 10 MHz and above map to DATA-USB.
#[test]
fn data_mode_for_high_hf_bands() {
    assert_eq!(data_mode_for_frequency(10_142_000.0), "DATA-USB"); // 30m
    assert_eq!(data_mode_for_frequency(14_070_000.0), "DATA-USB"); // 20m
    assert_eq!(data_mode_for_frequency(18_100_000.0), "DATA-USB"); // 17m
    assert_eq!(data_mode_for_frequency(21_080_000.0), "DATA-USB"); // 15m
    assert_eq!(data_mode_for_frequency(28_120_000.0), "DATA-USB"); // 10m
    assert_eq!(data_mode_for_frequency(10_000_000.0), "DATA-USB"); // exact threshold
}

/// MockRadio at 20m in wrong mode (LSB) → mode guard corrects to DATA-USB.
#[test]
fn mode_guard_corrects_mode_on_20m() {
    let mut radio = MockRadio::new();
    // Radio starts at 14.070 MHz / DATA-USB; put it in the wrong mode
    radio.set_mode("LSB").unwrap();
    assert_eq!(radio.get_mode().unwrap(), "LSB");

    // Simulate the ensure_data_mode logic
    let hz = radio.get_frequency().unwrap().as_hz();
    let target = data_mode_for_frequency(hz);
    let current = radio.get_mode().unwrap();
    if current != target {
        radio.set_mode(target).unwrap();
    }

    assert_eq!(radio.get_mode().unwrap(), "DATA-USB");
}

/// MockRadio on 40m in wrong mode (USB) → mode guard corrects to DATA-LSB.
#[test]
fn mode_guard_corrects_mode_on_40m() {
    let mut radio = MockRadio::new();
    radio.set_frequency(Frequency::hz(7_035_000.0)).unwrap();
    radio.set_mode("USB").unwrap();

    let hz = radio.get_frequency().unwrap().as_hz();
    let target = data_mode_for_frequency(hz);
    let current = radio.get_mode().unwrap();
    if current != target {
        radio.set_mode(target).unwrap();
    }

    assert_eq!(radio.get_mode().unwrap(), "DATA-LSB");
}

/// MockRadio already in DATA-USB on 20m → mode guard is a no-op.
#[test]
fn mode_guard_noop_when_mode_already_correct() {
    let mut radio = MockRadio::new(); // 14.070 MHz / DATA-USB by default

    let hz = radio.get_frequency().unwrap().as_hz();
    let target = data_mode_for_frequency(hz);
    let current = radio.get_mode().unwrap();

    // Should not need to change
    assert_eq!(current, target, "mode should already be correct");
    // (no set_mode call made — if we did, it would still be DATA-USB)
}

/// PTT state is preserved in AppState across multiple lock acquisitions.
#[test]
fn ptt_state_visible_through_appstate() {
    let state = connected_state();

    {
        let mut lock = state.radio.lock().unwrap();
        let radio = lock.as_mut().unwrap();
        assert!(!radio.is_transmitting());
        radio.ptt_on().unwrap();
        assert!(radio.is_transmitting());
    }

    // Re-acquire the lock — state must persist
    {
        let lock = state.radio.lock().unwrap();
        assert!(lock.as_ref().unwrap().is_transmitting());
    }
}
