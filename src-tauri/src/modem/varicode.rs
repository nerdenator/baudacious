//! Varicode encoding/decoding for PSK-31
//!
//! Varicode uses variable-length bit patterns for each character.
//! More common characters have shorter codes. Each code ends with "00".

/// Varicode encoder/decoder
pub struct Varicode;

impl Varicode {
    /// Encode a character to its Varicode bit pattern
    /// Returns None for unsupported characters (including non-ASCII)
    pub fn encode(ch: char) -> Option<&'static str> {
        // Reject non-ASCII up front: Varicode is ASCII-only (0x00–0x7E).
        // `ch as u8` would silently truncate chars > U+00FF (e.g. 'Ā' → 0x00 = NUL),
        // producing a wrong encoding instead of None.
        if !ch.is_ascii() {
            return None;
        }
        let code = match ch as u8 {
            0x00 => "1010101011",  // NUL
            0x01 => "1011011011",  // SOH
            0x02 => "1011101101",  // STX
            0x03 => "1101110111",  // ETX
            0x04 => "1011101011",  // EOT
            0x05 => "1101011111",  // ENQ
            0x06 => "1011101111",  // ACK
            0x07 => "1011111101",  // BEL
            0x08 => "1011111111",  // BS
            0x09 => "11101111",    // HT (tab)
            0x0A => "11101",       // LF (newline)
            0x0B => "1101101111",  // VT
            0x0C => "1011011101",  // FF
            0x0D => "11111",       // CR (carriage return)
            0x0E => "1101110101",  // SO
            0x0F => "1110101011",  // SI
            0x10 => "1011110111",  // DLE
            0x11 => "1011110101",  // DC1
            0x12 => "1110101101",  // DC2
            0x13 => "1110101111",  // DC3
            0x14 => "1101011011",  // DC4
            0x15 => "1101101011",  // NAK
            0x16 => "1101101101",  // SYN
            0x17 => "1101010111",  // ETB
            0x18 => "1101111011",  // CAN
            0x19 => "1101111101",  // EM
            0x1A => "1110110111",  // SUB
            0x1B => "1101010101",  // ESC
            0x1C => "1101011101",  // FS
            0x1D => "1110111011",  // GS
            0x1E => "1011111011",  // RS
            0x1F => "1101111111",  // US
            0x20 => "1",           // Space (most common = shortest)
            0x21 => "111111111",   // !
            0x22 => "101011111",   // "
            0x23 => "111110101",   // #
            0x24 => "111011011",   // $
            0x25 => "1011010101",  // %
            0x26 => "1010111011",  // &
            0x27 => "101111111",   // '
            0x28 => "11111011",    // (
            0x29 => "11110111",    // )
            0x2A => "101101111",   // *
            0x2B => "111011111",   // +
            0x2C => "1110101",     // ,
            0x2D => "110101",      // -
            0x2E => "1010111",     // .
            0x2F => "110101111",   // /
            0x30 => "10110111",    // 0
            0x31 => "10111101",    // 1
            0x32 => "11101101",    // 2
            0x33 => "11111111",    // 3
            0x34 => "101110111",   // 4
            0x35 => "101011011",   // 5
            0x36 => "101101011",   // 6
            0x37 => "110101101",   // 7
            0x38 => "110101011",   // 8
            0x39 => "110110111",   // 9
            0x3A => "11110101",    // :
            0x3B => "110111101",   // ;
            0x3C => "111101101",   // <
            0x3D => "1010101",     // =
            0x3E => "111010111",   // >
            0x3F => "1010101111",  // ?
            0x40 => "1010111101",  // @
            0x41 => "1111101",     // A
            0x42 => "11101011",    // B
            0x43 => "10101101",    // C
            0x44 => "10110101",    // D
            0x45 => "1110111",     // E
            0x46 => "11011011",    // F
            0x47 => "11111101",    // G
            0x48 => "101010101",   // H
            0x49 => "1111111",     // I
            0x4A => "111111101",   // J
            0x4B => "101111101",   // K
            0x4C => "11010111",    // L
            0x4D => "10111011",    // M
            0x4E => "11011101",    // N
            0x4F => "10101011",    // O
            0x50 => "11010101",    // P
            0x51 => "111011101",   // Q
            0x52 => "10101111",    // R
            0x53 => "1101111",     // S
            0x54 => "1101101",     // T
            0x55 => "101010111",   // U
            0x56 => "110110101",   // V
            0x57 => "101011101",   // W
            0x58 => "101110101",   // X
            0x59 => "101111011",   // Y
            0x5A => "1010101101",  // Z
            0x5B => "111110111",   // [
            0x5C => "111101111",   // backslash
            0x5D => "111111011",   // ]
            0x5E => "1010111111",  // ^
            0x5F => "101101101",   // _
            0x60 => "1011011111",  // `
            0x61 => "1011",        // a
            0x62 => "1011111",     // b
            0x63 => "101111",      // c
            0x64 => "101101",      // d
            0x65 => "11",          // e (most common letter = very short)
            0x66 => "111101",      // f
            0x67 => "1011011",     // g
            0x68 => "101011",      // h
            0x69 => "1101",        // i
            0x6A => "111101011",   // j
            0x6B => "10111111",    // k
            0x6C => "11011",       // l
            0x6D => "111011",      // m
            0x6E => "1111",        // n
            0x6F => "111",         // o
            0x70 => "111111",      // p
            0x71 => "110111111",   // q
            0x72 => "10101",       // r
            0x73 => "10111",       // s
            0x74 => "101",         // t
            0x75 => "110111",      // u
            0x76 => "1111011",     // v
            0x77 => "1101011",     // w
            0x78 => "11011111",    // x
            0x79 => "1011101",     // y
            0x7A => "111010101",   // z
            0x7B => "1010110111",  // {
            0x7C => "110111011",   // |
            0x7D => "1010110101",  // }
            0x7E => "1011010111",  // ~
            0x7F => "1110110101",  // DEL
            _ => return None,
        };
        Some(code)
    }

    /// Convert a bit string to actual bits
    pub fn bits_from_str(s: &str) -> Vec<bool> {
        s.chars().map(|c| c == '1').collect()
    }
}

/// Varicode decoder state machine
pub struct VaricodeDecoder {
    bit_buffer: u16,
    bit_count: u8,
    consecutive_zeros: u8,
}

impl VaricodeDecoder {
    pub fn new() -> Self {
        Self {
            bit_buffer: 0,
            bit_count: 0,
            consecutive_zeros: 0,
        }
    }

    /// Push a bit into the decoder, returns decoded character if complete.
    ///
    /// Key insight: zeros are *deferred* — we don't add them to the buffer
    /// immediately because they might be the "00" separator between characters.
    /// Only when a subsequent '1' arrives do we flush pending zeros into the
    /// buffer (confirming they were internal to the varicode pattern).
    pub fn push_bit(&mut self, bit: bool) -> Option<char> {
        if bit {
            // Flush any pending zeros — they're internal to the code, not separators
            for _ in 0..self.consecutive_zeros {
                self.bit_buffer <<= 1; // zero bit
                self.bit_count += 1;
            }
            self.consecutive_zeros = 0;

            // Add the '1' bit
            self.bit_buffer = (self.bit_buffer << 1) | 1;
            self.bit_count += 1;
        } else {
            self.consecutive_zeros += 1;

            if self.consecutive_zeros >= 2 && self.bit_count > 0 {
                // "00" found = character boundary. Buffer has the complete code.
                let ch = self.lookup_code();
                self.bit_buffer = 0;
                self.bit_count = 0;
                self.consecutive_zeros = 0;
                return ch;
            }
        }

        None
    }

    fn lookup_code(&self) -> Option<char> {
        // This is a simplified lookup - in practice you'd use a trie or hashmap
        // For now, do a linear search through all codes
        for ch in 0u8..=127 {
            if let Some(code_str) = Varicode::encode(ch as char) {
                let code_bits: u16 = code_str
                    .chars()
                    .fold(0u16, |acc, c| (acc << 1) | if c == '1' { 1 } else { 0 });
                let code_len = code_str.len() as u8;

                if code_len == self.bit_count && code_bits == self.bit_buffer {
                    return Some(ch as char);
                }
            }
        }
        None
    }

    pub fn reset(&mut self) {
        self.bit_buffer = 0;
        self.bit_count = 0;
        self.consecutive_zeros = 0;
    }
}

impl Default for VaricodeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varicode_decoder_default_matches_new() {
        let a = VaricodeDecoder::new();
        let b = VaricodeDecoder::default();
        assert_eq!(a.bit_count, b.bit_count);
        assert_eq!(a.bit_buffer, b.bit_buffer);
        assert_eq!(a.consecutive_zeros, b.consecutive_zeros);
    }

    #[test]
    fn test_encode_common_chars() {
        assert_eq!(Varicode::encode(' '), Some("1"));
        assert_eq!(Varicode::encode('e'), Some("11"));
        assert_eq!(Varicode::encode('t'), Some("101"));
        assert_eq!(Varicode::encode('\n'), Some("11101"));
    }

    #[test]
    fn test_decode_roundtrip() {
        let mut decoder = VaricodeDecoder::new();

        // Encode "test"
        let test_chars = ['t', 'e', 's', 't'];
        let mut all_bits = Vec::new();

        for ch in test_chars {
            let code = Varicode::encode(ch).unwrap();
            all_bits.extend(Varicode::bits_from_str(code));
            all_bits.push(false); // Add separator
            all_bits.push(false);
        }

        // Decode
        let mut decoded = String::new();
        for bit in all_bits {
            if let Some(ch) = decoder.push_bit(bit) {
                decoded.push(ch);
            }
        }

        assert_eq!(decoded, "test");
    }

    #[test]
    fn test_bits_from_str_all_ones() {
        let bits = Varicode::bits_from_str("111");
        assert_eq!(bits, vec![true, true, true]);
    }

    #[test]
    fn test_bits_from_str_all_zeros() {
        let bits = Varicode::bits_from_str("000");
        assert_eq!(bits, vec![false, false, false]);
    }

    #[test]
    fn test_bits_from_str_mixed() {
        let bits = Varicode::bits_from_str("101");
        assert_eq!(bits, vec![true, false, true]);
    }

    #[test]
    fn test_bits_from_str_empty() {
        let bits = Varicode::bits_from_str("");
        assert_eq!(bits, Vec::<bool>::new());
    }

    #[test]
    fn test_encode_char_a_uppercase() {
        assert_eq!(Varicode::encode('A'), Some("1111101"));
    }

    #[test]
    fn test_encode_char_z_uppercase() {
        assert_eq!(Varicode::encode('Z'), Some("1010101101"));
    }

    #[test]
    fn test_encode_digit_zero() {
        assert_eq!(Varicode::encode('0'), Some("10110111"));
    }

    #[test]
    fn test_encode_digit_nine() {
        assert_eq!(Varicode::encode('9'), Some("110110111"));
    }

    #[test]
    fn test_encode_nul_char() {
        assert_eq!(Varicode::encode('\0'), Some("1010101011"));
    }

    #[test]
    fn test_encode_space() {
        assert_eq!(Varicode::encode(' '), Some("1"));
    }

    #[test]
    fn test_encode_unsupported_char_returns_none() {
        // Characters above 0x7F are not in the Varicode table
        assert_eq!(Varicode::encode('é'), None);
    }

    #[test]
    fn test_encode_char_above_u00ff_returns_none_not_truncated() {
        // Regression: 'Ā' is U+0100. Before the is_ascii() guard, `ch as u8` would
        // truncate 0x100 → 0x00 (NUL) and return Some("1010101011") instead of None.
        assert_eq!(Varicode::encode('Ā'), None);
        // Emoji similarly must not wrap around to a valid code
        assert_eq!(Varicode::encode('😀'), None);
    }

    #[test]
    fn test_decoder_reset_clears_state() {
        let mut decoder = VaricodeDecoder::new();

        // Push some bits to build partial state
        decoder.push_bit(true);
        decoder.push_bit(true);
        decoder.push_bit(false);

        decoder.reset();

        // After reset, pushing "00" should not decode anything (buffer is empty)
        let r1 = decoder.push_bit(false);
        let r2 = decoder.push_bit(false);
        assert!(r1.is_none(), "Should not decode after reset+00 with no data");
        assert!(r2.is_none(), "Should not decode after reset+00 with no data");
    }

    #[test]
    fn test_decoder_reset_and_reuse() {
        let mut decoder = VaricodeDecoder::new();

        // Decode 'e' ("11" + "00")
        decoder.push_bit(true);
        decoder.push_bit(true);
        decoder.push_bit(false);
        let ch = decoder.push_bit(false);
        assert_eq!(ch, Some('e'));

        // Reset and decode 't' ("101" + "00")
        decoder.reset();
        decoder.push_bit(true);
        decoder.push_bit(false);
        decoder.push_bit(true);
        decoder.push_bit(false);
        let ch2 = decoder.push_bit(false);
        assert_eq!(ch2, Some('t'));
    }
}
