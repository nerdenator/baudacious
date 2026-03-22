//! Pure encoding: CatCommand → CAT wire string.
//!
//! No I/O, no side effects. Easy to unit-test without any serial port.
//!
//! In Python terms this is like a pure function module — you pass in a
//! command value and get back the exact bytes to send to the radio.

use super::{CatCommand, MODE_TABLE};

/// Encode a CatCommand into the FT-991A wire string (including the `;` terminator).
pub fn encode(cmd: &CatCommand) -> String {
    use CatCommand::*;
    match cmd {
        GetFrequencyA => "FA;".into(),
        SetFrequencyA(hz) => format!("FA{hz:09};"),
        GetMode => "MD0;".into(),
        SetMode(name) => {
            let code = MODE_TABLE
                .iter()
                .find(|(_, n)| *n == name.as_str())
                .map(|(c, _)| *c)
                .unwrap_or_else(|| {
                    log::warn!("encode: unknown mode '{name}', falling back to DATA-USB");
                    "C"
                });
            format!("MD0{code};")
        }
        PttOff => "TX0;".into(),
        PttOn => "TX1;".into(),
        GetTxPower => "PC;".into(),
        SetTxPower(w) => format!("PC{w:03};"),
        GetSignalStrength => "SM0;".into(),
        GetStatus => "IF;".into(),
        BandSelect(code) => format!("BS{code:02};"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CatCommand::*;

    #[test]
    fn encode_get_frequency_a() {
        assert_eq!(encode(&GetFrequencyA), "FA;");
    }

    #[test]
    fn encode_set_frequency_20m() {
        // FT-991A uses 9-digit format for frequency
        assert_eq!(encode(&SetFrequencyA(14_070_000)), "FA014070000;");
    }

    #[test]
    fn encode_set_frequency_40m() {
        assert_eq!(encode(&SetFrequencyA(7_035_000)), "FA007035000;");
    }

    #[test]
    fn encode_set_frequency_zero_padded() {
        assert_eq!(encode(&SetFrequencyA(1_800_000)), "FA001800000;");
    }

    #[test]
    fn encode_get_mode() {
        assert_eq!(encode(&GetMode), "MD0;");
    }

    #[test]
    fn encode_set_mode_data_usb() {
        assert_eq!(encode(&SetMode("DATA-USB".into())), "MD0C;");
    }

    #[test]
    fn encode_set_mode_usb() {
        assert_eq!(encode(&SetMode("USB".into())), "MD02;");
    }

    #[test]
    fn encode_set_mode_lsb() {
        assert_eq!(encode(&SetMode("LSB".into())), "MD01;");
    }

    #[test]
    fn encode_set_mode_unknown_falls_back_to_data_usb() {
        // Unknown modes fall back to DATA-USB (code "C")
        assert_eq!(encode(&SetMode("GIBBERISH".into())), "MD0C;");
    }

    #[test]
    fn encode_ptt_on() {
        assert_eq!(encode(&PttOn), "TX1;");
    }

    #[test]
    fn encode_ptt_off() {
        assert_eq!(encode(&PttOff), "TX0;");
    }

    #[test]
    fn encode_get_tx_power() {
        assert_eq!(encode(&GetTxPower), "PC;");
    }

    #[test]
    fn encode_set_tx_power_25w() {
        assert_eq!(encode(&SetTxPower(25)), "PC025;");
    }

    #[test]
    fn encode_set_tx_power_100w() {
        assert_eq!(encode(&SetTxPower(100)), "PC100;");
    }

    #[test]
    fn encode_set_tx_power_zero() {
        assert_eq!(encode(&SetTxPower(0)), "PC000;");
    }

    #[test]
    fn encode_get_signal_strength() {
        assert_eq!(encode(&GetSignalStrength), "SM0;");
    }

    #[test]
    fn encode_get_status() {
        assert_eq!(encode(&GetStatus), "IF;");
    }

    #[test]
    fn encode_band_select() {
        assert_eq!(encode(&BandSelect(5)), "BS05;");
    }

    #[test]
    fn encode_band_select_zero_padded() {
        // Single-digit band codes must be zero-padded to 2 digits
        assert_eq!(encode(&BandSelect(1)), "BS01;");
        assert_eq!(encode(&BandSelect(10)), "BS10;");
    }

    #[test]
    fn encode_all_modes_roundtrip() {
        use super::super::MODE_TABLE;
        for (code, name) in MODE_TABLE {
            let encoded = encode(&SetMode(name.to_string()));
            assert_eq!(encoded, format!("MD0{code};"), "Roundtrip failed for mode '{name}'");
        }
    }
}
