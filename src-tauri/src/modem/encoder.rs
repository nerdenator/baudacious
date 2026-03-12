//! PSK-31 Encoder — converts text to BPSK-31 audio samples
//!
//! Pipeline: text → Varicode bits → BPSK modulation → audio samples
//!
//! BPSK-31 encodes data by phase: a '1' bit keeps the same phase,
//! a '0' bit flips the phase by 180°. Think of it like Python's
//! `itertools.accumulate` over the bit stream, toggling a boolean.
//!
//! Raised cosine shaping spans the *boundary* between symbols so that the
//! phase flip always occurs at zero amplitude — no spectral splatter.
//! Each symbol's envelope is assembled from two independent half-windows:
//!
//!   prev symbol               |  curr symbol
//!   ──────────────────────────|──────────────────────────
//!   2nd half: falling (1→0)   |  1st half: rising (0→1)
//!   (only if curr will flip)  |  (only if curr flipped)
//!
//! The second half of each symbol depends on the *next* bit (look-ahead).

use crate::dsp::nco::Nco;
use crate::modem::varicode::Varicode;

/// At 48 kHz sample rate and 31.25 baud, each symbol is exactly 1536 samples
const SAMPLES_PER_SYMBOL: usize = 1536;

/// Number of idle (phase-change) bits before data — lets the receiver lock on
const PREAMBLE_BITS: usize = 32;

/// Number of idle bits after data — clean ramp-down
const POSTAMBLE_BITS: usize = 32;

/// PSK-31 encoder: text in, audio samples out
pub struct Psk31Encoder {
    sample_rate: u32,
    carrier_freq: f64,
}

impl Psk31Encoder {
    pub fn new(sample_rate: u32, carrier_freq: f64) -> Self {
        Self {
            sample_rate,
            carrier_freq,
        }
    }

    /// Encode a text message into BPSK-31 audio samples.
    ///
    /// Returns a Vec<f32> of audio samples ready for playback at 48 kHz.
    pub fn encode(&self, text: &str) -> Vec<f32> {
        let bits = self.text_to_bits(text);
        self.bits_to_samples(&bits)
    }

    /// Convert text to a complete bit stream: preamble + varicode + postamble
    ///
    /// In BPSK-31, a '0' bit = phase change, '1' bit = no change.
    /// Varicode separators are '00' (two phase changes between characters).
    /// Preamble/postamble are all zeros (continuous phase changes).
    fn text_to_bits(&self, text: &str) -> Vec<bool> {
        let mut bits = Vec::new();

        // Preamble: continuous phase changes for receiver sync
        for _ in 0..PREAMBLE_BITS {
            bits.push(false); // 0 = phase change
        }

        // Encode each character
        for ch in text.chars() {
            if let Some(code_str) = Varicode::encode(ch) {
                let code_bits = Varicode::bits_from_str(code_str);
                bits.extend(code_bits);
                // Inter-character separator: two zeros
                bits.push(false);
                bits.push(false);
            }
            // Skip unsupported characters silently
        }

        // Postamble: clean ramp-down
        for _ in 0..POSTAMBLE_BITS {
            bits.push(false);
        }

        bits
    }

    /// Convert a bit stream to BPSK-modulated audio samples.
    ///
    /// Each symbol is split into two 768-sample halves. The envelope shape of
    /// each half is chosen independently:
    ///
    ///   first half  — rising (0→1) if this symbol flips phase, else flat (1)
    ///   second half — falling (1→0) if the *next* symbol will flip, else flat (1)
    ///
    /// The phase flip still fires at t=0 of the symbol, but by then the previous
    /// symbol's falling second half has already ramped the amplitude to zero, so
    /// the discontinuity is inaudible.
    fn bits_to_samples(&self, bits: &[bool]) -> Vec<f32> {
        let mut nco = Nco::new(self.carrier_freq, self.sample_rate as f64);
        let half = SAMPLES_PER_SYMBOL / 2; // 768 samples

        // Precompute the two half-window shapes once.
        //
        // Both are derived from |cos(π·t)| over a full symbol period, split at
        // the midpoint so that rising[0]==0, rising[767]≈1, falling[0]==1,
        // falling[767]≈0 — they meet at zero exactly at the symbol boundary.
        //
        // rising[k]  = |cos(π · (k + half) / SAMPLES_PER_SYMBOL)|  (second half of V)
        // falling[k] = |cos(π ·  k          / SAMPLES_PER_SYMBOL)|  (first  half of V)
        let rising: Vec<f32> = (0..half)
            .map(|k| {
                let t = (k + half) as f32 / SAMPLES_PER_SYMBOL as f32;
                (std::f32::consts::PI * t).cos().abs()
            })
            .collect();

        let falling: Vec<f32> = (0..half)
            .map(|k| {
                let t = k as f32 / SAMPLES_PER_SYMBOL as f32;
                (std::f32::consts::PI * t).cos().abs()
            })
            .collect();

        let flat = vec![1.0f32; half];

        let total_samples = bits.len() * SAMPLES_PER_SYMBOL;
        let mut samples = Vec::with_capacity(total_samples);

        for (i, &bit) in bits.iter().enumerate() {
            let phase_change = !bit; // bit=false → phase change (BPSK convention)
            let next_phase_change = i + 1 < bits.len() && !bits[i + 1];

            // Flip phase at t=0. The previous symbol's falling second half has
            // already driven amplitude to zero, so there is no discontinuity.
            if phase_change {
                nco.adjust_phase(std::f64::consts::PI);
            }

            // First half: rising from zero if this symbol flipped, else constant.
            let first_half: &[f32] = if phase_change { &rising } else { &flat };

            // Second half: falling toward zero if the next symbol will flip,
            // else constant — the next symbol's rising half will then take over.
            let second_half: &[f32] = if next_phase_change { &falling } else { &flat };

            for &env in first_half.iter().chain(second_half.iter()) {
                samples.push(nco.next() * env);
            }
        }

        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modem::varicode::VaricodeDecoder;

    #[test]
    fn test_encode_empty_text() {
        let encoder = Psk31Encoder::new(48000, 1500.0);
        let samples = encoder.encode("");

        // Should have preamble + postamble only
        let expected_bits = PREAMBLE_BITS + POSTAMBLE_BITS;
        let expected_samples = expected_bits * SAMPLES_PER_SYMBOL;
        assert_eq!(samples.len(), expected_samples);
    }

    #[test]
    fn test_encode_single_char() {
        let encoder = Psk31Encoder::new(48000, 1500.0);
        let samples = encoder.encode("e");

        // 'e' = "11" (2 bits) + "00" separator (2 bits)
        // Total: 32 preamble + 2 char + 2 separator + 32 postamble = 68 bits
        let expected_samples = 68 * SAMPLES_PER_SYMBOL;
        assert_eq!(samples.len(), expected_samples);
    }

    #[test]
    fn test_encode_known_text_bit_count() {
        let encoder = Psk31Encoder::new(48000, 1500.0);

        // "CQ" = C="10101101" (8 bits) + 00 + Q="111011101" (9 bits) + 00
        // Total data bits: 8 + 2 + 9 + 2 = 21
        // Total: 32 + 21 + 32 = 85 bits
        let samples = encoder.encode("CQ");
        let expected_samples = 85 * SAMPLES_PER_SYMBOL;
        assert_eq!(samples.len(), expected_samples);
    }

    #[test]
    fn test_samples_in_valid_range() {
        let encoder = Psk31Encoder::new(48000, 1500.0);
        let samples = encoder.encode("TEST");

        for (i, &s) in samples.iter().enumerate() {
            assert!(
                s >= -1.0 && s <= 1.0,
                "Sample {} out of range: {}",
                i,
                s
            );
        }
    }

    #[test]
    fn test_preamble_is_not_silent() {
        let encoder = Psk31Encoder::new(48000, 1500.0);
        let samples = encoder.encode("A");

        // The preamble should have audible carrier, not silence
        let preamble_samples = &samples[..PREAMBLE_BITS * SAMPLES_PER_SYMBOL];
        let max_amplitude: f32 = preamble_samples
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);

        assert!(
            max_amplitude > 0.1,
            "Preamble seems silent: max amplitude = {}",
            max_amplitude
        );
    }

    #[test]
    fn test_encode_decode_loopback() {
        // Encode "HI" → detect phase transitions → decode with VaricodeDecoder
        let encoder = Psk31Encoder::new(48000, 1500.0);
        let samples = encoder.encode("HI");

        // Extract bits by detecting phase changes between symbols.
        // We compare the sign of the carrier at a consistent point in each
        // symbol (quarter-way through, to avoid envelope nulls).
        let check_offset = SAMPLES_PER_SYMBOL / 4;
        let num_symbols = samples.len() / SAMPLES_PER_SYMBOL;

        let mut bits = Vec::new();
        let mut prev_sign = samples[check_offset] >= 0.0;

        for sym_idx in 1..num_symbols {
            let sample_idx = sym_idx * SAMPLES_PER_SYMBOL + check_offset;
            let current_sign = samples[sample_idx] >= 0.0;
            // Phase change = bit 0, no change = bit 1
            bits.push(current_sign == prev_sign);
            prev_sign = current_sign;
        }

        // Skip preamble bits (they're all zeros / phase changes)
        // and decode the data portion
        let data_bits = &bits[PREAMBLE_BITS - 1..]; // -1 because we lost the first bit in diff detection

        let mut decoder = VaricodeDecoder::new();
        let mut decoded = String::new();

        for &bit in data_bits {
            if let Some(ch) = decoder.push_bit(bit) {
                decoded.push(ch);
            }
        }

        assert!(
            decoded.contains("HI"),
            "Expected decoded text to contain 'HI', got: '{}'",
            decoded
        );
    }
}
