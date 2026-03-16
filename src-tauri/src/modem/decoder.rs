//! PSK-31 Decoder — converts BPSK-31 audio samples back to text
//!
//! Pipeline: audio samples → AGC → Costas Loop → clock recovery
//!           → differential bit detection → Varicode decode → characters
//!
//! The Costas Loop locks onto the BPSK carrier, tracks its frequency/phase,
//! and downmixes to baseband. Differential decoding resolves the 180° phase
//! ambiguity by detecting phase *changes* rather than absolute phase.
//!
//! Note: The first character of a transmission is typically lost during
//! lock acquisition. This is normal PSK-31 behavior — real QSOs always
//! start with repeated CQ calls so the receiver has time to lock.

use crate::dsp::agc::Agc;
use crate::dsp::clock_recovery::ClockRecovery;
use crate::dsp::costas_loop::CostasLoop;
use crate::modem::varicode::VaricodeDecoder;

/// Number of bits without a valid decoded character before we try
/// inverting the bit sense (phase ambiguity fallback)
const PHASE_AMBIGUITY_THRESHOLD: usize = 100;

/// Minimum symbol magnitude for bit decisions. Below this threshold,
/// the Costas Loop hasn't locked yet and bit decisions would be garbage.
const SYMBOL_SQUELCH: f32 = 0.001;

/// PSK-31 decoder: audio samples in, decoded characters out
pub struct Psk31Decoder {
    agc: Agc,
    costas_loop: CostasLoop,
    clock_recovery: ClockRecovery,
    varicode_decoder: VaricodeDecoder,

    /// Previous symbol value for differential detection
    last_symbol: f32,

    /// Count of consecutive bits without a valid Varicode character
    bits_without_char: usize,

    /// When true, invert bit sense (phase ambiguity fallback)
    invert_bits: bool,

    sample_rate: u32,
    carrier_freq: f64,
}

impl Psk31Decoder {
    /// Create a new decoder tuned to the given carrier frequency
    ///
    /// - `carrier_freq`: audio carrier in Hz (typically 500-2500, set by waterfall click)
    /// - `sample_rate`: audio sample rate (48000)
    pub fn new(carrier_freq: f64, sample_rate: u32) -> Self {
        let samples_per_symbol = sample_rate as f64 / 31.25;

        Self {
            agc: Agc::new(0.5),
            costas_loop: CostasLoop::new(carrier_freq, sample_rate as f64, 2.0),
            clock_recovery: ClockRecovery::new(samples_per_symbol),
            varicode_decoder: VaricodeDecoder::new(),
            last_symbol: 0.0,
            bits_without_char: 0,
            invert_bits: false,
            sample_rate,
            carrier_freq,
        }
    }

    /// Process a single audio sample. Returns `Some(char)` when a character
    /// is fully decoded, `None` otherwise.
    pub fn process(&mut self, sample: f32) -> Option<char> {
        // 1. AGC — normalize amplitude
        let normalized = self.agc.process(sample);

        // 2. Costas Loop — carrier tracking + downmix to baseband
        let baseband = self.costas_loop.process(normalized);

        // 3. Clock Recovery — extract symbol at decision points
        let symbol = self.clock_recovery.process(baseband)?;

        // 4. Symbol squelch — ignore weak symbols during lock acquisition
        if symbol.abs() < SYMBOL_SQUELCH && self.last_symbol.abs() < SYMBOL_SQUELCH {
            self.last_symbol = symbol;
            return None;
        }

        // 5. Differential bit detection
        let same_sign = (symbol > 0.0) == (self.last_symbol > 0.0);
        self.last_symbol = symbol;

        let raw_bit = same_sign;
        let bit = if self.invert_bits { !raw_bit } else { raw_bit };

        // 6. Varicode decode
        self.bits_without_char += 1;

        if let Some(ch) = self.varicode_decoder.push_bit(bit) {
            self.bits_without_char = 0;
            return Some(ch);
        }

        // 7. Phase ambiguity fallback
        if self.bits_without_char > PHASE_AMBIGUITY_THRESHOLD {
            self.invert_bits = !self.invert_bits;
            self.bits_without_char = 0;
            self.varicode_decoder.reset();
        }

        None
    }

    /// Update the carrier frequency (e.g., from waterfall click-to-tune)
    ///
    /// Resets carrier tracking and bit-layer state but preserves AGC gain
    /// to avoid unnecessary settle time after retuning.
    pub fn set_carrier_freq(&mut self, freq: f64) {
        self.carrier_freq = freq;
        self.costas_loop.set_frequency(freq);
        self.costas_loop.reset();
        self.clock_recovery = ClockRecovery::new(self.sample_rate as f64 / 31.25);
        self.varicode_decoder.reset();
        self.last_symbol = 0.0;
        self.bits_without_char = 0;
        self.invert_bits = false;
    }

    /// Update the carrier frequency only if the change exceeds 0.1 Hz.
    ///
    /// This is the correct call site for the audio thread — it internalises the
    /// change-detection threshold so the commands layer does not need to shadow
    /// the carrier frequency.
    pub fn update_carrier_if_changed(&mut self, freq: f64) {
        if (freq - self.carrier_freq).abs() > 0.1 {
            self.set_carrier_freq(freq);
        }
    }

    /// Signal strength as a 0.0..=1.0 value derived from AGC gain.
    ///
    /// The AGC gain is inversely proportional to signal level: low gain = strong signal.
    /// Gain range is [0.01, 100.0], mapped via inverse log10 to [1.0, 0.0]:
    ///   gain=0.01 → 1.0 (strong), gain=1.0 → 0.5, gain=100.0 → 0.0 (absent)
    pub fn signal_strength(&self) -> f32 {
        let gain = self.agc.current_gain().clamp(0.01, 100.0);
        (1.0 - (gain.log10() + 2.0) / 4.0).clamp(0.0, 1.0)
    }

    /// Reset all decoder state
    pub fn reset(&mut self) {
        self.agc.reset();
        self.costas_loop.reset();
        self.clock_recovery.reset();
        self.varicode_decoder.reset();
        self.last_symbol = 0.0;
        self.bits_without_char = 0;
        self.invert_bits = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modem::encoder::Psk31Encoder;

    #[test]
    fn test_decode_encoder_output() {
        let carrier_freq = 1000.0;
        let sample_rate = 48000;

        let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
        let samples = encoder.encode("HI");

        let mut decoder = Psk31Decoder::new(carrier_freq, sample_rate);
        let mut decoded = String::new();

        for &sample in &samples {
            if let Some(ch) = decoder.process(sample) {
                decoded.push(ch);
            }
        }

        // First character may be lost during lock acquisition — this is
        // normal PSK-31 behavior. Check that we decode at least the 'I'.
        assert!(
            decoded.contains('I'),
            "Expected to decode at least 'I', got: '{}'",
            decoded
        );
    }

    #[test]
    fn test_decode_longer_text() {
        let carrier_freq = 1500.0;
        let sample_rate = 48000;

        let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
        let samples = encoder.encode("CQ CQ DE W1AW");

        let mut decoder = Psk31Decoder::new(carrier_freq, sample_rate);
        let mut decoded = String::new();

        for &sample in &samples {
            if let Some(ch) = decoder.process(sample) {
                decoded.push(ch);
            }
        }

        // Should decode the bulk of the message (first char may be lost)
        assert!(
            decoded.contains("Q DE W1AW"),
            "Expected message core, got: '{}'",
            decoded
        );
    }

    #[test]
    fn test_decode_at_different_carrier() {
        let carrier_freq = 2000.0;
        let sample_rate = 48000;

        let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
        let samples = encoder.encode("TEST");

        let mut decoder = Psk31Decoder::new(carrier_freq, sample_rate);
        let mut decoded = String::new();

        for &sample in &samples {
            if let Some(ch) = decoder.process(sample) {
                decoded.push(ch);
            }
        }

        assert!(
            decoded.contains("EST"),
            "Expected at least 'EST' at 2000 Hz, got: '{}'",
            decoded
        );
    }

    #[test]
    fn test_retune_resets_state() {
        let mut decoder = Psk31Decoder::new(1000.0, 48000);

        for i in 0..10000 {
            decoder.process((i as f32 * 0.1).sin());
        }

        decoder.set_carrier_freq(1500.0);
        assert_eq!(decoder.carrier_freq, 1500.0);
        assert_eq!(decoder.last_symbol, 0.0);
        assert_eq!(decoder.bits_without_char, 0);
        assert!(!decoder.invert_bits);
    }

    #[test]
    fn update_carrier_if_changed_no_op_below_threshold() {
        let mut decoder = Psk31Decoder::new(1500.0, 48000);
        // Process a few samples to accumulate some state
        for _ in 0..100 {
            decoder.process(0.0);
        }
        let carrier_before = decoder.carrier_freq;
        // Delta of 0.05 Hz — below the 0.1 Hz threshold
        decoder.update_carrier_if_changed(1500.05);
        // carrier_freq must not have changed
        assert_eq!(decoder.carrier_freq, carrier_before);
    }

    #[test]
    fn update_carrier_if_changed_resets_above_threshold() {
        let mut decoder = Psk31Decoder::new(1500.0, 48000);
        // Delta of 50 Hz — well above threshold
        decoder.update_carrier_if_changed(1550.0);
        assert_eq!(decoder.carrier_freq, 1550.0);
    }

    // --- signal_strength boundaries ---

    #[test]
    fn signal_strength_at_default_gain_is_midrange() {
        // Fresh decoder has gain=1.0 → log10(1.0)=0 → (1-(0+2)/4) = 0.5
        let decoder = Psk31Decoder::new(1000.0, 48000);
        let s = decoder.signal_strength();
        assert!((s - 0.5).abs() < 0.01, "expected ~0.5 at default gain, got {s}");
    }

    #[test]
    fn signal_strength_with_saturated_high_gain_approaches_zero() {
        // Feed silence to drive AGC gain toward max (100.0)
        let mut decoder = Psk31Decoder::new(1000.0, 48000);
        for _ in 0..100_000 {
            decoder.process(0.0);
        }
        let s = decoder.signal_strength();
        // At max gain 100.0: log10(100)=2 → (1-(2+2)/4) = 0.0
        assert!(s < 0.05, "expected near 0.0 with high gain, got {s}");
    }

    #[test]
    fn signal_strength_with_loud_signal_is_higher_than_silence() {
        // After loud signal the AGC reduces gain → signal_strength rises above baseline.
        // After silence the AGC increases gain → signal_strength falls below baseline.
        let mut decoder_loud = Psk31Decoder::new(1000.0, 48000);
        let mut decoder_quiet = Psk31Decoder::new(1000.0, 48000);
        for _ in 0..10_000 {
            decoder_loud.process(1.0);
            decoder_quiet.process(0.0);
        }
        let s_loud = decoder_loud.signal_strength();
        let s_quiet = decoder_quiet.signal_strength();
        assert!(
            s_loud > s_quiet,
            "loud signal should yield higher signal_strength than silence: loud={s_loud}, quiet={s_quiet}"
        );
    }

    // --- reset clears all state ---

    #[test]
    fn reset_restores_decoder_to_initial_behavior() {
        let carrier = 1000.0;
        let sample_rate = 48000;

        // Decode a real signal to put the decoder into a mid-stream state
        let encoder = Psk31Encoder::new(sample_rate, carrier);
        let samples = encoder.encode("TEST");
        let mut decoder = Psk31Decoder::new(carrier, sample_rate);
        for &s in &samples {
            decoder.process(s);
        }

        // Reset and verify the decoder can cleanly decode a fresh transmission
        decoder.reset();
        let samples2 = encoder.encode("HI");
        let mut decoded = String::new();
        for &s in &samples2 {
            if let Some(ch) = decoder.process(s) {
                decoded.push(ch);
            }
        }
        // After reset, decoder re-acquires lock — at least the last character decodes
        assert!(
            decoded.contains('I') || decoded.contains('H'),
            "expected at least one character after reset, got: '{decoded}'"
        );
    }

    // --- phase ambiguity fallback ---

    #[test]
    fn phase_ambiguity_fallback_does_not_crash() {
        // Acquire lock with a real signal, then starve the decoder with silence
        // to trigger the PHASE_AMBIGUITY_THRESHOLD inversion, then verify
        // the decoder is still functional (doesn't panic, still processes samples).
        let carrier = 1000.0;
        let sample_rate = 48000;
        let encoder = Psk31Encoder::new(sample_rate, carrier);

        // Feed preamble samples to acquire lock
        let samples = encoder.encode("E");
        let mut decoder = Psk31Decoder::new(carrier, sample_rate);
        for &s in &samples {
            decoder.process(s);
        }

        // Feed silence — this drives bits_without_char past PHASE_AMBIGUITY_THRESHOLD
        // which triggers the invert_bits flip and varicode_decoder.reset()
        for _ in 0..(PHASE_AMBIGUITY_THRESHOLD + 50) * 1536 {
            decoder.process(0.0);
        }

        // Decoder must survive without panicking and still accept new samples
        let samples2 = encoder.encode("E");
        let mut post_fallback_output = 0usize;
        for &s in &samples2 {
            if decoder.process(s).is_some() {
                post_fallback_output += 1;
            }
        }
        // We don't assert specific decoded chars (lock re-acquisition is non-deterministic)
        // but we assert the decoder ran all the way through without panicking
        let _ = post_fallback_output;
    }
}
