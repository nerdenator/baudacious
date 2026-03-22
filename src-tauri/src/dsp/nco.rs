//! Numerically Controlled Oscillator

use std::f64::consts::PI;

/// Numerically Controlled Oscillator for generating carrier signals
pub struct Nco {
    phase: f64,
    phase_increment: f64,
    sample_rate: f64,
}

impl Nco {
    /// Create a new NCO with the given frequency and sample rate
    pub fn new(frequency: f64, sample_rate: f64) -> Self {
        let phase_increment = 2.0 * PI * frequency / sample_rate;
        Self {
            phase: 0.0,
            phase_increment,
            sample_rate,
        }
    }

    /// Set the oscillator frequency
    pub fn set_frequency(&mut self, frequency: f64) {
        self.phase_increment = 2.0 * PI * frequency / self.sample_rate;
    }

    /// Get the current frequency
    pub fn frequency(&self) -> f64 {
        self.phase_increment * self.sample_rate / (2.0 * PI)
    }

    /// Adjust phase by a delta (used by PLLs for frequency correction)
    pub fn adjust_phase(&mut self, delta: f64) {
        self.phase += delta;
        self.wrap_phase();
    }

    /// Generate the next I/Q sample pair (cos, sin)
    pub fn next_iq(&mut self) -> (f32, f32) {
        let i = self.phase.cos() as f32;
        let q = self.phase.sin() as f32;
        self.phase += self.phase_increment;
        self.wrap_phase();
        (i, q)
    }

    /// Generate the next real sample (cosine only)
    pub fn next(&mut self) -> f32 {
        let sample = self.phase.cos() as f32;
        self.phase += self.phase_increment;
        self.wrap_phase();
        sample
    }

    /// Reset phase to zero
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn wrap_phase(&mut self) {
        while self.phase >= 2.0 * PI {
            self.phase -= 2.0 * PI;
        }
        while self.phase < 0.0 {
            self.phase += 2.0 * PI;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nco_frequency() {
        let mut nco = Nco::new(1000.0, 48000.0);

        // Generate one full cycle worth of samples
        let samples_per_cycle = 48000.0 / 1000.0; // 48 samples
        let mut samples = Vec::new();

        for _ in 0..(samples_per_cycle as usize * 2) {
            samples.push(nco.next());
        }

        // Check that we have roughly 2 complete cycles
        let zero_crossings: usize = samples
            .windows(2)
            .filter(|w| (w[0] >= 0.0 && w[1] < 0.0) || (w[0] < 0.0 && w[1] >= 0.0))
            .count();

        // 2 cycles = 4 zero crossings
        assert_eq!(zero_crossings, 4);
    }

    #[test]
    fn test_set_frequency_updates_phase_increment() {
        let mut nco = Nco::new(1000.0, 48000.0);
        nco.set_frequency(2000.0);
        let freq = nco.frequency();
        assert!((freq - 2000.0).abs() < 0.001, "Expected 2000 Hz, got {}", freq);
    }

    #[test]
    fn test_frequency_getter_matches_constructor() {
        let nco = Nco::new(1500.0, 48000.0);
        let freq = nco.frequency();
        assert!((freq - 1500.0).abs() < 0.001, "Expected 1500 Hz, got {}", freq);
    }

    #[test]
    fn test_adjust_phase_shifts_output() {
        let mut nco_ref = Nco::new(1000.0, 48000.0);
        let mut nco_shifted = Nco::new(1000.0, 48000.0);

        // Shift by pi — cosine(x + pi) = -cosine(x)
        nco_shifted.adjust_phase(PI);

        let ref_sample = nco_ref.next();
        let shifted_sample = nco_shifted.next();

        assert!(
            (ref_sample + shifted_sample).abs() < 0.01,
            "Phase shift of pi should invert the signal: ref={}, shifted={}",
            ref_sample,
            shifted_sample
        );
    }

    #[test]
    fn test_next_iq_returns_cos_sin_pair() {
        let mut nco = Nco::new(0.0, 48000.0); // 0 Hz — phase stays at 0
        let (i, q) = nco.next_iq();
        // At phase=0: cos(0)=1, sin(0)=0
        assert!((i - 1.0).abs() < 0.001, "I should be cos(0)=1, got {}", i);
        assert!(q.abs() < 0.001, "Q should be sin(0)=0, got {}", q);
    }

    #[test]
    fn test_reset_restores_phase_to_zero() {
        let mut nco = Nco::new(1000.0, 48000.0);

        // Advance to some arbitrary phase
        for _ in 0..100 {
            nco.next();
        }

        nco.reset();

        // After reset, first sample should be cos(0) = 1.0
        let sample = nco.next();
        assert!(
            (sample - 1.0).abs() < 0.001,
            "After reset, first sample should be 1.0, got {}",
            sample
        );
    }

    #[test]
    fn test_phase_wraps_after_adjust() {
        let mut nco = Nco::new(1000.0, 48000.0);

        // Adjust by a large amount — phase should still produce valid output
        nco.adjust_phase(100.0 * PI);

        let (i, q) = nco.next_iq();
        // Values must be in [-1, 1]
        assert!(i >= -1.0 && i <= 1.0, "I out of range: {}", i);
        assert!(q >= -1.0 && q <= 1.0, "Q out of range: {}", q);
    }

    #[test]
    fn test_phase_wraps_negative_adjust() {
        let mut nco = Nco::new(1000.0, 48000.0);

        // Adjust by a large negative amount
        nco.adjust_phase(-100.0 * PI);

        let (i, q) = nco.next_iq();
        assert!(i >= -1.0 && i <= 1.0, "I out of range after negative adjust: {}", i);
        assert!(q >= -1.0 && q <= 1.0, "Q out of range after negative adjust: {}", q);
    }
}
