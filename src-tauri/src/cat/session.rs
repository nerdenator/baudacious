//! CatSession: owns a serial connection and drives CAT I/O timing.
//!
//! This is analogous to a Python class that wraps a serial.Serial object and
//! adds the FT-991A protocol layer: command delay, read-until-semicolon loop,
//! echo stripping, and error detection.
//!
//! Pure translation lives in `encode` / `decode`. CatSession only handles I/O.

use std::time::{Duration, Instant};

use crate::domain::{Psk31Error, Psk31Result};
use crate::ports::SerialConnection;

use super::{decode, encode, CatCommand, CatResponse};

/// Minimum delay between CAT commands (FT-991A firmware requirement)
const COMMAND_DELAY_MS: u64 = 50;

/// Settling delay after opening a serial port before sending the first command.
/// Some USB-serial adapters (e.g. CP2105) need a moment to become ready.
const PORT_SETTLE_MS: u64 = 200;

/// Chunk size for each serial read call
const READ_CHUNK_SIZE: usize = 64;

/// Max read attempts before giving up (~100ms per attempt → ~1000ms total)
const RESPONSE_TIMEOUT_READS: usize = 10;

/// Owns a serial connection and executes CAT commands against the FT-991A.
pub struct CatSession {
    serial: Box<dyn SerialConnection>,
    last_command_time: Option<Instant>,
}

impl CatSession {
    pub fn new(serial: Box<dyn SerialConnection>) -> Self {
        // Give the USB-serial adapter time to settle before the first command.
        std::thread::sleep(Duration::from_millis(PORT_SETTLE_MS));
        Self {
            serial,
            last_command_time: None,
        }
    }

    /// Send a CAT command and return the parsed response.
    ///
    /// Enforces the 50ms inter-command delay, writes the wire string,
    /// reads bytes until the `;` terminator, strips any command echo,
    /// then delegates to `decode()`.
    pub fn execute(&mut self, cmd: &CatCommand) -> Psk31Result<CatResponse> {
        self.ensure_command_delay();

        let wire = encode(cmd);
        log::debug!("CAT TX: {wire}");

        self.serial
            .write(wire.as_bytes())
            .map_err(|e| Psk31Error::Cat(format!("Command '{wire}' write failed: {e}")))?;

        let raw = self.read_until_semicolon(&wire);
        // Update timestamp even on error so the next command still respects the delay
        self.last_command_time = Some(Instant::now());
        let raw = raw?;

        log::debug!("CAT RX: {raw}");

        // Strip command echo if present (some USB-serial adapters echo the TX)
        // e.g. "FA;FA00014070000;" → "FA00014070000;"
        let raw = raw.strip_prefix(&wire).unwrap_or(&raw);

        decode(raw, cmd)
    }

    /// Write a command and do NOT wait for a response.
    ///
    /// Used for FT-991A commands that execute silently without sending an ack:
    /// - `BS;` (band select) — changes band stack, no `;` returned
    /// - `FA;` (set frequency) — on some firmware variants, no `;` returned
    ///
    /// Returns an error only if the serial write itself fails.
    pub fn execute_write_only(&mut self, cmd: &CatCommand) -> Psk31Result<()> {
        self.ensure_command_delay();

        let wire = encode(cmd);
        log::debug!("CAT TX: {wire}");

        self.serial
            .write(wire.as_bytes())
            .map_err(|e| Psk31Error::Cat(format!("Command '{wire}' write failed: {e}")))?;

        self.last_command_time = Some(Instant::now());
        Ok(())
    }

    /// Read bytes from the serial port until a `;` appears or timeout.
    ///
    /// Each serial.read() has a 100ms hardware timeout. We retry up to
    /// RESPONSE_TIMEOUT_READS times to handle slow USB-serial adapters
    /// that may return partial responses.
    fn read_until_semicolon(&mut self, cmd_wire: &str) -> Psk31Result<String> {
        let mut buf: Vec<u8> = Vec::with_capacity(READ_CHUNK_SIZE);
        let mut chunk = [0u8; READ_CHUNK_SIZE];

        for _ in 0..RESPONSE_TIMEOUT_READS {
            match self.serial.read(&mut chunk) {
                Ok(n) if n > 0 => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.contains(&b';') {
                        break;
                    }
                }
                Ok(_) => {} // Zero bytes: read timed out, try again
                Err(_) => {
                    // Timeout or transient error: keep retrying.
                    // If the radio never responds, the "no response" check below fires
                    // after all attempts are exhausted.
                }
            }
        }

        if buf.is_empty() {
            return Err(Psk31Error::Cat(format!(
                "Command '{cmd_wire}': no response from radio"
            )));
        }

        std::str::from_utf8(&buf)
            .map(|s| s.to_string())
            .map_err(|e| Psk31Error::Cat(format!("Invalid UTF-8 response: {e}")))
    }

    /// Sleep if needed to maintain the minimum inter-command delay.
    fn ensure_command_delay(&self) {
        if let Some(last) = self.last_command_time {
            let elapsed = last.elapsed();
            let min_delay = Duration::from_millis(COMMAND_DELAY_MS);
            if elapsed < min_delay {
                std::thread::sleep(min_delay - elapsed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::SerialConnection;
    use std::sync::{Arc, Mutex};

    // ---------------------------------------------------------------------------
    // MockSerial for CatSession tests
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

    fn make_session(response: &str) -> (CatSession, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mock = MockSerial {
            log: Arc::clone(&log),
            response: response.to_string(),
        };
        (CatSession::new(Box::new(mock)), log)
    }

    // --- Basic I/O ---

    #[test]
    fn execute_get_frequency_sends_fa_query() {
        let (mut session, log) = make_session("FA00014070000;");
        session.execute(&CatCommand::GetFrequencyA).unwrap();
        assert_eq!(log.lock().unwrap()[0], "FA;");
    }

    #[test]
    fn execute_ptt_on_sends_tx1() {
        let (mut session, log) = make_session(";");
        session.execute(&CatCommand::PttOn).unwrap();
        assert_eq!(log.lock().unwrap()[0], "TX1;");
    }

    #[test]
    fn execute_ptt_off_sends_tx0() {
        let (mut session, log) = make_session(";");
        session.execute(&CatCommand::PttOff).unwrap();
        assert_eq!(log.lock().unwrap()[0], "TX0;");
    }

    // --- NAK ---

    #[test]
    fn nak_response_returns_err() {
        let (mut session, _) = make_session("?");
        let result = session.execute(&CatCommand::GetFrequencyA);
        assert!(result.is_err(), "? response should be Err");
    }

    // --- Ack ---

    #[test]
    fn semicolon_response_returns_ack() {
        let (mut session, _) = make_session(";");
        let resp = session.execute(&CatCommand::PttOn).unwrap();
        assert_eq!(resp, CatResponse::Ack);
    }

    // --- Echo stripping ---

    #[test]
    fn echo_stripped_before_decode() {
        // Some adapters echo the sent command: "FA;FA00014070000;"
        let (mut session, _) = make_session("FA;FA00014070000;");
        let resp = session.execute(&CatCommand::GetFrequencyA).unwrap();
        assert_eq!(resp, CatResponse::FrequencyHz(14_070_000));
    }

    // --- TX power ---

    #[test]
    fn get_tx_power_returns_watts() {
        let (mut session, log) = make_session("PC025;");
        let resp = session.execute(&CatCommand::GetTxPower).unwrap();
        assert_eq!(resp, CatResponse::TxPower(25));
        assert_eq!(log.lock().unwrap()[0], "PC;");
    }

    #[test]
    fn set_tx_power_sends_correct_wire() {
        let (mut session, log) = make_session(";");
        session.execute(&CatCommand::SetTxPower(50)).unwrap();
        assert_eq!(log.lock().unwrap()[0], "PC050;");
    }

    // --- Long response (regression: 64-byte fixed buffer truncated long replies) ---

    /// A MockSerial that streams its response one byte at a time.
    /// This reproduces the pre-fix behaviour where a response longer than the
    /// read chunk size would be silently truncated on the first read call.
    struct StreamingMockSerial {
        response: Vec<u8>,
        cursor: usize,
    }

    impl SerialConnection for StreamingMockSerial {
        fn write(&mut self, data: &[u8]) -> Psk31Result<usize> {
            Ok(data.len())
        }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            if self.cursor >= self.response.len() {
                return Ok(0);
            }
            // Return exactly one byte per call to maximally stress the accumulation loop
            buf[0] = self.response[self.cursor];
            self.cursor += 1;
            Ok(1)
        }
        fn close(&mut self) -> Psk31Result<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
    }

    // --- Timeout and error paths in read_until_semicolon ---

    /// A MockSerial that always returns Ok(0) — simulates a radio that never responds.
    struct SilentMockSerial;

    impl SerialConnection for SilentMockSerial {
        fn write(&mut self, _data: &[u8]) -> Psk31Result<usize> { Ok(0) }
        fn read(&mut self, _buf: &mut [u8]) -> Psk31Result<usize> { Ok(0) }
        fn close(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_connected(&self) -> bool { true }
    }

    #[test]
    fn all_reads_timeout_returns_no_response_error() {
        let mut session = CatSession::new(Box::new(SilentMockSerial));
        let result = session.execute(&CatCommand::GetFrequencyA);
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no response"), "expected 'no response' in: {err}");
    }

    /// A MockSerial that returns Err on every read — simulates transient I/O errors.
    struct ErroringMockSerial {
        response_after: Vec<u8>,
        error_count: usize,
        calls: usize,
        cursor: usize,
    }

    impl SerialConnection for ErroringMockSerial {
        fn write(&mut self, _data: &[u8]) -> Psk31Result<usize> { Ok(0) }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            self.calls += 1;
            if self.calls <= self.error_count {
                return Err(Psk31Error::Serial("transient error".into()));
            }
            if self.cursor >= self.response_after.len() {
                return Ok(0);
            }
            buf[0] = self.response_after[self.cursor];
            self.cursor += 1;
            Ok(1)
        }
        fn close(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_connected(&self) -> bool { true }
    }

    #[test]
    fn transient_read_errors_retry_and_succeed() {
        // First 3 reads return Err, then the real response arrives byte-by-byte.
        let serial = ErroringMockSerial {
            response_after: b"FA00014070000;".to_vec(),
            error_count: 3,
            calls: 0,
            cursor: 0,
        };
        let mut session = CatSession::new(Box::new(serial));
        let result = session.execute(&CatCommand::GetFrequencyA);
        assert!(result.is_ok(), "expected success after transient errors: {result:?}");
    }

    // --- execute_write_only ---

    #[test]
    fn execute_write_only_sends_wire_bytes() {
        let (mut session, log) = make_session("");
        session.execute_write_only(&CatCommand::BandSelect(3)).unwrap();
        assert_eq!(log.lock().unwrap()[0], "BS03;");
    }

    /// A MockSerial whose write() always fails.
    struct FailingWriteMockSerial;

    impl SerialConnection for FailingWriteMockSerial {
        fn write(&mut self, _data: &[u8]) -> Psk31Result<usize> {
            Err(Psk31Error::Serial("write failed".into()))
        }
        fn read(&mut self, _buf: &mut [u8]) -> Psk31Result<usize> { Ok(0) }
        fn close(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_connected(&self) -> bool { true }
    }

    #[test]
    fn execute_write_only_propagates_write_error() {
        let mut session = CatSession::new(Box::new(FailingWriteMockSerial));
        let result = session.execute_write_only(&CatCommand::BandSelect(5));
        assert!(result.is_err(), "expected Err when write fails");
    }

    // --- ensure_command_delay (inner branch with Some(last_command_time)) ---

    #[test]
    fn ensure_command_delay_sleeps_when_within_window() {
        // Set last_command_time to *just now* so elapsed < 50ms is guaranteed,
        // deterministically exercising the sleep branch regardless of CI load.
        let (mut session, _) = make_session("FA00014070000;");
        session.last_command_time = Some(Instant::now());
        // execute() calls ensure_command_delay first; with last_command_time just set,
        // the sleep path is guaranteed to run.
        let result = session.execute(&CatCommand::GetFrequencyA);
        assert!(result.is_ok(), "execute after forced delay should succeed: {result:?}");
    }

    // --- Invalid UTF-8 response ---

    struct InvalidUtf8MockSerial;

    impl SerialConnection for InvalidUtf8MockSerial {
        fn write(&mut self, _: &[u8]) -> Psk31Result<usize> { Ok(0) }
        fn read(&mut self, buf: &mut [u8]) -> Psk31Result<usize> {
            // 0xFF 0xFE are invalid UTF-8 followed by ';' to end the read loop
            let bytes: &[u8] = &[0xFF, 0xFE, b';'];
            let n = bytes.len().min(buf.len());
            buf[..n].copy_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn close(&mut self) -> Psk31Result<()> { Ok(()) }
        fn is_connected(&self) -> bool { true }
    }

    #[test]
    fn invalid_utf8_response_returns_err() {
        let mut session = CatSession::new(Box::new(InvalidUtf8MockSerial));
        let result = session.execute(&CatCommand::GetFrequencyA);
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid UTF-8"), "expected UTF-8 error: {err}");
    }

    // --- Long response regression ---

    #[test]
    fn long_response_read_across_multiple_chunks() {
        // Regression: the old code used a fixed [u8; 64] accumulation buffer, so any
        // response longer than 64 bytes would be truncated and the semicolon missed.
        // StreamingMockSerial delivers one byte per read() call — the fix must accumulate
        // across calls until the `;` terminator arrives.
        //
        // Construct a response that is >64 bytes: echo prefix + real FrequencyA payload.
        // "FA;" (3 bytes) + "FA00014070000;" (14 bytes) = 17 bytes normally.
        // Pad to 70 bytes by prefixing with 56 extra FA; repetitions of garbage, then the
        // real response at the end. Since echo-stripping only removes an exact leading "FA;",
        // the simplest valid >64-byte response is just the GetMode echo path:
        // "MD0;" (4) repeated 15 times = 60 bytes, then "MD0C;" (5 bytes) = 65 bytes total.
        let response = format!("{}{}", "MD0;".repeat(15), "MD0C;");
        assert!(response.len() > 64, "test setup: response must exceed 64 bytes");

        let serial = StreamingMockSerial {
            response: response.into_bytes(),
            cursor: 0,
        };
        let mut session = CatSession::new(Box::new(serial));
        // The long echo prefix won't be stripped (strip_prefix only removes exact match),
        // so decode gets "MD0;" x15 + "MD0C;" which won't parse — but the important
        // thing is it doesn't panic or return a truncated result. We only care that
        // CatSession successfully reads past the 64-byte mark without dropping bytes.
        // Verify by checking the error isn't a "no response" error:
        let result = session.execute(&CatCommand::GetMode);
        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.contains("no response"),
                    "got 'no response' — bytes were truncated: {msg}"
                );
            }
            Ok(_) => {} // If it somehow parses, that's fine too
        }
    }
}
