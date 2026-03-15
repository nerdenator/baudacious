//! Pure decoding: CAT wire string + command context → CatResponse.
//!
//! No I/O, no side effects. The `cmd` parameter tells us which fields
//! to expect in the response — the FT-991A uses the same prefix for
//! queries and replies so we need the context to know what we're parsing.

use crate::domain::{Psk31Error, Psk31Result, RadioStatus};

use super::{CatCommand, CatResponse, MODE_TABLE};

/// Decode a raw response string from the radio into a typed CatResponse.
///
/// `response` is the string received after stripping any command echo.
/// `cmd` is the command that was sent, used to pick the right parser.
///
/// Returns `Err` if the response is `"?"` (radio NAK) or cannot be parsed.
pub fn decode(response: &str, cmd: &CatCommand) -> Psk31Result<CatResponse> {
    use CatCommand::*;

    // The radio returns "?" when it doesn't understand or rejects a command.
    if response.trim_end_matches(';') == "?" || response == "?" {
        return Err(Psk31Error::Cat(format!(
            "Radio NAK for command {cmd:?}: response was '?'"
        )));
    }

    match cmd {
        GetFrequencyA => parse_frequency(response),
        SetFrequencyA(_) => expect_ack(response, cmd),
        GetMode => parse_mode(response),
        SetMode(_) => expect_ack(response, cmd),
        PttOn | PttOff => expect_ack(response, cmd),
        GetTxPower => parse_tx_power(response),
        SetTxPower(_) => expect_ack(response, cmd),
        GetSignalStrength => parse_signal_strength(response),
        GetStatus => parse_status(response),
        // BandSelect is write-only — never decoded, but must be covered for exhaustiveness.
        BandSelect(_) => expect_ack(response, cmd),
    }
}

/// Parse `"FA00014070000;"` or `"FA007073900;"` → `FrequencyHz(N)`
///
/// The FT-991A returns variable-width frequency strings (9 or 11 digits depending
/// on firmware/band).  We parse all digits after the `FA` prefix rather than
/// assuming a fixed width.
fn parse_frequency(response: &str) -> Psk31Result<CatResponse> {
    let trimmed = response.trim().trim_end_matches(';');
    if !trimmed.starts_with("FA") || trimmed.len() < 3 {
        return Err(Psk31Error::Cat(format!(
            "Invalid frequency response: '{response}'"
        )));
    }
    let digits = &trimmed[2..];
    let hz = digits
        .parse::<u64>()
        .map_err(|e| Psk31Error::Cat(format!("Failed to parse frequency '{digits}': {e}")))?;
    Ok(CatResponse::FrequencyHz(hz))
}

/// Parse `"MD0C;"` → `Mode("DATA-USB")`
fn parse_mode(response: &str) -> Psk31Result<CatResponse> {
    let trimmed = response.trim().trim_end_matches(';');
    if !trimmed.starts_with("MD0") || trimmed.len() < 4 {
        return Err(Psk31Error::Cat(format!(
            "Invalid mode response: '{response}'"
        )));
    }
    let code = &trimmed[3..4];
    MODE_TABLE
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| CatResponse::Mode(name.to_string()))
        .ok_or_else(|| Psk31Error::Cat(format!("Unknown mode code: '{code}'")))
}

/// Parse `"PC050;"` → `TxPower(50)`
fn parse_tx_power(response: &str) -> Psk31Result<CatResponse> {
    let trimmed = response.trim().trim_end_matches(';');
    if !trimmed.starts_with("PC") || trimmed.len() < 5 {
        return Err(Psk31Error::Cat(format!(
            "Invalid TX power response: '{response}'"
        )));
    }
    let digits = &trimmed[2..5];
    let watts = digits
        .parse::<u32>()
        .map_err(|e| Psk31Error::Cat(format!("Failed to parse TX power '{digits}': {e}")))?;
    Ok(CatResponse::TxPower(watts))
}

/// Parse `"SM00015;"` → `SignalStrength(0.5)`  (15 / 30 = 0.5)
///
/// Format: `"SM0"` + 4-digit value (0000–0030) + `";"`
fn parse_signal_strength(response: &str) -> Psk31Result<CatResponse> {
    let trimmed = response.trim().trim_end_matches(';');
    if !trimmed.starts_with("SM0") || trimmed.len() < 7 {
        return Err(Psk31Error::Cat(format!(
            "Invalid S-meter response: '{response}'"
        )));
    }
    let digits = &trimmed[3..7];
    let raw: u32 = digits.parse().map_err(|e| {
        Psk31Error::Cat(format!("Failed to parse S-meter value '{digits}': {e}"))
    })?;
    Ok(CatResponse::SignalStrength(raw.min(30) as f32 / 30.0))
}

/// Parse `"IF{body};"` → `Status(RadioStatus)`
///
/// The FT-991A has two known IF response body lengths depending on firmware:
///
/// **37-char format** (full):
/// ```text
/// [0..11]  VFO-A frequency, Hz, 11-digit zero-padded decimal
/// [11..16] blank (clarifier display, 5 chars)
/// [16..21] RIT/XIT offset: sign char + 4 decimal digits
/// [21]     RIT on/off  (0=off, 1=on)
/// [22]     XIT on/off  (0=off, 1=on)
/// [23..25] memory channel (2 chars, blank in VFO mode)
/// [25]     VFO/MEM indicator
/// [26]     TX status (0=RX, 1=TX, 2=TX tune)
/// [27..29] mode code (2 chars with leading '0': "0C"=DATA-USB)
/// [29..31] function, scan (ignored)
/// [31]     split (0=simplex, 1=split)
/// [32..37] tone, CTCSS, shift etc. (ignored)
/// ```
///
/// **25-char format** (compact — omits blank padding, memory, and tail):
/// ```text
/// [0..11]  VFO-A frequency, Hz, 11-digit zero-padded decimal
/// [11]     indicator (1 char)
/// [12..17] RIT/XIT offset: sign char + 4 decimal digits
/// [17]     RIT on/off  (0=off, 1=on)
/// [18]     XIT on/off  (0=off, 1=on)
/// [19]     mode code (1 char, no leading '0': "C"=DATA-USB)
/// [20]     VFO/MEM indicator
/// [21]     TX status (0=RX, 1=TX, 2=TX tune)
/// [22]     function (ignored)
/// [23]     scan (ignored)
/// [24]     split (0=simplex, 1=split)
/// ```
fn parse_status(response: &str) -> Psk31Result<CatResponse> {
    let trimmed = response.trim().trim_end_matches(';');
    if !trimmed.starts_with("IF") {
        return Err(Psk31Error::Cat(format!(
            "Invalid IF response (missing 'IF' prefix): '{response}'"
        )));
    }
    let body = &trimmed[2..]; // strip "IF" prefix

    if body.len() >= 37 {
        parse_status_full(body, response)
    } else if body.len() >= 25 {
        parse_status_compact(body, response)
    } else {
        Err(Psk31Error::Cat(format!(
            "IF response body too short: {} chars (need 25 or 37): '{response}'",
            body.len()
        )))
    }
}

/// Parse the 37-char full IF body format.
fn parse_status_full(body: &str, _response: &str) -> Psk31Result<CatResponse> {
    let freq_str = &body[0..11];
    let frequency_hz: u64 = freq_str.parse().map_err(|e| {
        Psk31Error::Cat(format!("IF: failed to parse frequency '{freq_str}': {e}"))
    })?;

    let rit_offset_hz = parse_rit_offset(&body[16..21]);
    let rit_enabled = body.as_bytes()[21] == b'1';
    let is_transmitting = matches!(body.as_bytes()[26], b'1' | b'2');

    let mode_code_padded = &body[27..29];
    let mode_code = mode_code_padded.trim_start_matches('0');
    let mode = lookup_mode(mode_code, mode_code_padded);

    let split = body.as_bytes().get(31).map(|&b| b != b'0').unwrap_or(false);

    Ok(CatResponse::Status(RadioStatus {
        frequency_hz,
        mode,
        is_transmitting,
        rit_offset_hz,
        rit_enabled,
        split,
    }))
}

/// Parse the 25-char compact IF body format.
///
/// This firmware variant prefixes the body with 3 indicator chars (`00X`) before
/// the 9-digit VFO frequency.  All subsequent field positions remain unchanged.
fn parse_status_compact(body: &str, _response: &str) -> Psk31Result<CatResponse> {
    // Frequency: 9 digits at [3..12] (preceded by a 3-char prefix, e.g. "001")
    let freq_str = &body[3..12];
    let frequency_hz: u64 = freq_str.parse().map_err(|e| {
        Psk31Error::Cat(format!("IF: failed to parse frequency '{freq_str}': {e}"))
    })?;

    // RIT offset: [12..17] — sign char at [12] + 4 decimal digits at [13..17]
    let rit_offset_hz = parse_rit_offset(&body[12..17]);
    let rit_enabled = body.as_bytes()[17] == b'1';
    let is_transmitting = matches!(body.as_bytes()[21], b'1' | b'2');

    // Mode: [19] — single char, no leading '0'
    let mode_code = &body[19..20];
    let mode = lookup_mode(mode_code, mode_code);

    let split = body.as_bytes().get(24).map(|&b| b != b'0').unwrap_or(false);

    Ok(CatResponse::Status(RadioStatus {
        frequency_hz,
        mode,
        is_transmitting,
        rit_offset_hz,
        rit_enabled,
        split,
    }))
}

/// Look up a mode name from the MODE_TABLE. Falls back to DATA-USB on unknown codes.
fn lookup_mode(mode_code: &str, mode_code_for_log: &str) -> String {
    if mode_code.is_empty() {
        return "DATA-USB".to_string();
    }
    MODE_TABLE
        .iter()
        .find(|(c, _)| *c == mode_code)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| {
            log::warn!("IF: unknown mode code '{mode_code_for_log}', defaulting to DATA-USB");
            "DATA-USB".to_string()
        })
}

/// Parse a 5-char RIT offset string into signed Hz.
///
/// Accepts formats like `"+1000"`, `"-0500"`, `"00000"` (no offset).
/// Returns 0 on any parse failure (non-fatal: RIT offset is advisory).
fn parse_rit_offset(s: &str) -> i32 {
    if s.len() < 2 {
        return 0;
    }
    match s.as_bytes()[0] {
        b'+' => s[1..].parse::<i32>().unwrap_or(0),
        b'-' => s[1..].parse::<i32>().unwrap_or(0).wrapping_neg(),
        _ => s.parse::<i32>().unwrap_or(0),
    }
}

/// For commands where the radio only returns `";"` (or empty Ack).
fn expect_ack(response: &str, cmd: &CatCommand) -> Psk31Result<CatResponse> {
    let trimmed = response.trim();
    if trimmed == ";" || trimmed.is_empty() {
        Ok(CatResponse::Ack)
    } else {
        Err(Psk31Error::Cat(format!(
            "Expected Ack (';') for {cmd:?}, got: '{response}'"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CatCommand::*;

    // --- NAK ---

    #[test]
    fn nak_returns_err() {
        assert!(decode("?", &GetFrequencyA).is_err());
        assert!(decode("?;", &GetFrequencyA).is_err());
        assert!(decode("?", &PttOn).is_err());
    }

    // --- GetFrequencyA ---

    #[test]
    fn decode_frequency_20m() {
        assert_eq!(
            decode("FA00014070000;", &GetFrequencyA).unwrap(),
            CatResponse::FrequencyHz(14_070_000)
        );
    }

    #[test]
    fn decode_frequency_40m() {
        assert_eq!(
            decode("FA00007074000;", &GetFrequencyA).unwrap(),
            CatResponse::FrequencyHz(7_074_000)
        );
    }

    #[test]
    fn decode_frequency_invalid_prefix() {
        assert!(decode("FB00014070000;", &GetFrequencyA).is_err());
    }

    #[test]
    fn decode_frequency_9digit() {
        // FT-991A returns 9-digit responses for HF frequencies
        assert_eq!(
            decode("FA007073900;", &GetFrequencyA).unwrap(),
            CatResponse::FrequencyHz(7_073_900)
        );
    }

    #[test]
    fn decode_frequency_too_short() {
        // "FA;" with nothing after the prefix is invalid
        assert!(decode("FA;", &GetFrequencyA).is_err());
    }

    // --- SetFrequencyA ---

    #[test]
    fn decode_set_frequency_ack() {
        assert_eq!(
            decode(";", &SetFrequencyA(14_070_000)).unwrap(),
            CatResponse::Ack
        );
    }

    // --- GetMode ---

    #[test]
    fn decode_mode_data_usb() {
        assert_eq!(
            decode("MD0C;", &GetMode).unwrap(),
            CatResponse::Mode("DATA-USB".into())
        );
    }

    #[test]
    fn decode_mode_usb() {
        assert_eq!(
            decode("MD02;", &GetMode).unwrap(),
            CatResponse::Mode("USB".into())
        );
    }

    #[test]
    fn decode_mode_lsb() {
        assert_eq!(
            decode("MD01;", &GetMode).unwrap(),
            CatResponse::Mode("LSB".into())
        );
    }

    #[test]
    fn decode_mode_unknown_code() {
        assert!(decode("MD0Z;", &GetMode).is_err());
    }

    #[test]
    fn decode_mode_too_short() {
        assert!(decode("MD;", &GetMode).is_err());
    }

    // --- SetMode ---

    #[test]
    fn decode_set_mode_ack() {
        assert_eq!(
            decode(";", &SetMode("DATA-USB".into())).unwrap(),
            CatResponse::Ack
        );
    }

    // --- PTT ---

    #[test]
    fn decode_ptt_on_ack() {
        assert_eq!(decode(";", &PttOn).unwrap(), CatResponse::Ack);
    }

    #[test]
    fn decode_ptt_off_ack() {
        assert_eq!(decode(";", &PttOff).unwrap(), CatResponse::Ack);
    }

    // --- GetTxPower ---

    #[test]
    fn decode_tx_power_50w() {
        assert_eq!(
            decode("PC050;", &GetTxPower).unwrap(),
            CatResponse::TxPower(50)
        );
    }

    #[test]
    fn decode_tx_power_100w() {
        assert_eq!(
            decode("PC100;", &GetTxPower).unwrap(),
            CatResponse::TxPower(100)
        );
    }

    #[test]
    fn decode_tx_power_invalid() {
        assert!(decode("PCXXX;", &GetTxPower).is_err());
        assert!(decode("PC;", &GetTxPower).is_err());
    }

    // --- SetTxPower ---

    #[test]
    fn decode_set_tx_power_ack() {
        assert_eq!(decode(";", &SetTxPower(25)).unwrap(), CatResponse::Ack);
    }

    // --- GetSignalStrength ---

    #[test]
    fn decode_signal_strength_half() {
        // 15 / 30 = 0.5
        assert_eq!(
            decode("SM00015;", &GetSignalStrength).unwrap(),
            CatResponse::SignalStrength(0.5)
        );
    }

    #[test]
    fn decode_signal_strength_zero() {
        assert_eq!(
            decode("SM00000;", &GetSignalStrength).unwrap(),
            CatResponse::SignalStrength(0.0)
        );
    }

    #[test]
    fn decode_signal_strength_max() {
        // 30 / 30 = 1.0
        assert_eq!(
            decode("SM00030;", &GetSignalStrength).unwrap(),
            CatResponse::SignalStrength(1.0)
        );
    }

    #[test]
    fn decode_signal_strength_clamps_above_30() {
        // Any value > 30 is clamped to 30 → 1.0
        // (shouldn't happen in practice, but be defensive)
        let r = decode("SM00030;", &GetSignalStrength).unwrap();
        assert_eq!(r, CatResponse::SignalStrength(1.0));
    }

    #[test]
    fn decode_signal_strength_too_short() {
        assert!(decode("SM0;", &GetSignalStrength).is_err());
    }

    // --- GetStatus (IF;) ---

    /// Build a valid 37-char IF response body for testing.
    ///
    /// Matches the byte layout documented in `parse_status`.
    fn make_if_response(freq: u64, mode: &str, tx: bool, rit_en: bool, rit_offset: i32, split: bool) -> String {
        let freq_str = format!("{freq:011}");
        let code = MODE_TABLE
            .iter()
            .find(|(_, n)| *n == mode)
            .map(|(c, _)| c)
            .unwrap_or(&"C");
        let mode_padded = format!("0{code}");
        let rit_sign = if rit_offset < 0 { '-' } else { '+' };
        let rit_abs = rit_offset.unsigned_abs();
        let rit_str = format!("{rit_sign}{rit_abs:04}");
        let rit_on = if rit_en { '1' } else { '0' };
        let tx_char = if tx { '1' } else { '0' };
        let split_char = if split { '1' } else { '0' };
        // [0..11]=freq [11..16]=blank [16..21]=rit_str [21]=rit_on [22]=XIT_off
        // [23..25]=mem [25]=VFO [26]=tx [27..29]=mode [29..31]=fn+scan [31]=split [32..37]=tail
        let body = format!(
            "{freq_str}     {rit_str}{rit_on}0  0{tx_char}{mode_padded}00{split_char}00000"
        );
        assert_eq!(body.len(), 37, "make_if_response: body must be 37 chars, got {}", body.len());
        format!("IF{body};")
    }

    #[test]
    fn decode_if_basic_20m_data_usb() {
        let response = make_if_response(14_070_000, "DATA-USB", false, false, 0, false);
        let s = match decode(&response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert_eq!(s.frequency_hz, 14_070_000);
        assert_eq!(s.mode, "DATA-USB");
        assert!(!s.is_transmitting);
        assert!(!s.rit_enabled);
        assert_eq!(s.rit_offset_hz, 0);
        assert!(!s.split);
    }

    #[test]
    fn decode_if_40m_data_lsb_transmitting() {
        let response = make_if_response(7_035_000, "DATA-LSB", true, false, 0, false);
        let s = match decode(&response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert_eq!(s.frequency_hz, 7_035_000);
        assert_eq!(s.mode, "DATA-LSB");
        assert!(s.is_transmitting);
    }

    #[test]
    fn decode_if_rit_positive() {
        let response = make_if_response(14_070_000, "DATA-USB", false, true, 500, false);
        let s = match decode(&response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert!(s.rit_enabled);
        assert_eq!(s.rit_offset_hz, 500);
    }

    #[test]
    fn decode_if_rit_negative() {
        let response = make_if_response(14_070_000, "DATA-USB", false, true, -250, false);
        let s = match decode(&response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert!(s.rit_enabled);
        assert_eq!(s.rit_offset_hz, -250);
    }

    #[test]
    fn decode_if_split_on() {
        let response = make_if_response(14_070_000, "DATA-USB", false, false, 0, true);
        let s = match decode(&response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert!(s.split);
    }

    #[test]
    fn decode_if_compact_25char() {
        // Actual response observed from FT-991A firmware (compact format, no blank padding).
        // Body: 001007073900+000000C00000 (25 chars)
        // Layout: prefix(3)="001" + freq(9)="007073900" + RIT(5)="+0000" +
        //         RIT_on(1)="0" + XIT_on(1)="0" + mode(1)="C" + indicator(1)="0" +
        //         TX(1)="0" + tail(3)="000"
        let response = "IF001007073900+000000C00000;";
        let s = match decode(response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert_eq!(s.frequency_hz, 7_073_900); // 9-digit freq at body[3..12]
        assert_eq!(s.mode, "DATA-USB");
        assert!(!s.is_transmitting);
        assert!(!s.rit_enabled);
        assert_eq!(s.rit_offset_hz, 0);
        assert!(!s.split);
    }

    #[test]
    fn decode_if_compact_80m_data_lsb() {
        // Observed from FT-991A on 80m: IF001003579300+000000800000;
        // freq(9)="003579300" = 3_579_300 Hz, mode "8" = DATA-LSB
        let response = "IF001003579300+000000800000;";
        let s = match decode(response, &GetStatus).unwrap() {
            CatResponse::Status(s) => s,
            _ => panic!("expected Status"),
        };
        assert_eq!(s.frequency_hz, 3_579_300);
        assert_eq!(s.mode, "DATA-LSB");
        assert!(!s.is_transmitting);
    }

    #[test]
    fn decode_if_too_short() {
        assert!(decode("IF12345;", &GetStatus).is_err());
    }

    #[test]
    fn decode_if_missing_prefix() {
        // 39-char string that doesn't start with "IF" should error
        let body: String = std::iter::repeat('0').take(37).collect();
        assert!(decode(&format!("XX{body};"), &GetStatus).is_err());
    }

    // --- Mode roundtrip ---

    #[test]
    fn decode_all_modes_roundtrip() {
        for (code, name) in MODE_TABLE {
            let response = format!("MD0{code};");
            let decoded = decode(&response, &GetMode).unwrap();
            assert_eq!(
                decoded,
                CatResponse::Mode(name.to_string()),
                "Roundtrip failed for mode code '{code}'"
            );
        }
    }
}
