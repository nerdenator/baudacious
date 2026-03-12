//! Symbol timing recovery using Mueller-Muller timing error detector
//!
//! Think of it like a metronome that self-adjusts: it counts samples between
//! "beats" (symbol decision points) and uses the M&M formula to nudge the
//! beat timing earlier or later so it lands at the optimal sampling instant.

/// Symbol clock recovery using Mueller-Muller timing error detector
pub struct ClockRecovery {
    samples_per_symbol: f64,
    omega: f64,        // Current samples-per-symbol estimate
    gain_omega: f64,   // Timing gain (how fast omega adapts)
    last_symbol: f32,  // Previous symbol decision-point value
    sample_count: f64,
}

impl ClockRecovery {
    /// Create a new clock recovery module
    ///
    /// For PSK-31 at 48kHz: samples_per_symbol = 48000 / 31.25 = 1536
    pub fn new(samples_per_symbol: f64) -> Self {
        Self {
            samples_per_symbol,
            omega: samples_per_symbol,
            gain_omega: 0.001,
            last_symbol: 0.0,
            // Start the counter at the half-symbol offset so the first decision
            // fires at sample (samples_per_symbol/2 - 1), i.e. the envelope peak
            // for a correct PSK-31 signal where transitions straddle symbol boundaries.
            sample_count: samples_per_symbol / 2.0,
        }
    }

    /// Process a sample, returns Some(symbol_value) at symbol decision points
    ///
    /// Uses Mueller-Muller timing error detection to adaptively track
    /// the optimal sampling instant. Returns None for most samples,
    /// Some(symbol_value) when we hit a decision point (~once per 1536 samples).
    pub fn process(&mut self, sample: f32) -> Option<f32> {
        self.sample_count += 1.0;

        // Check if we've reached a symbol decision point
        if self.sample_count >= self.omega {
            self.sample_count -= self.omega;

            // Mueller-Muller timing error detector
            // Compares the current sample at the decision point with the
            // midpoint sample (last_sample before decision) and previous decision.
            // e = d_{k-1} * x_k - d_k * x_{k-1}
            let decision = if sample >= 0.0 { 1.0f32 } else { -1.0 };
            let last_decision = if self.last_symbol >= 0.0 { 1.0f32 } else { -1.0 };
            let timing_error = last_decision * sample - decision * self.last_symbol;

            // Update omega (samples per symbol estimate) with very gentle adaptation
            self.omega += self.gain_omega * timing_error as f64;

            // Clamp omega to ±10% of nominal (prevents runaway)
            self.omega = self.omega.clamp(
                self.samples_per_symbol * 0.9,
                self.samples_per_symbol * 1.1,
            );

            self.last_symbol = sample;

            Some(sample)
        } else {
            None
        }
    }

    /// Reset the clock recovery state
    pub fn reset(&mut self) {
        self.omega = self.samples_per_symbol;
        self.last_symbol = 0.0;
        self.sample_count = self.samples_per_symbol / 2.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outputs_at_symbol_rate() {
        // Feed 10 symbols worth of samples, expect ~10 decision points
        let sps = 1536.0;
        let mut cr = ClockRecovery::new(sps);
        let mut decisions = 0;

        for i in 0..(10.0 * sps) as usize {
            // Simple square wave: positive for first half of symbol, negative for second
            let symbol_phase = (i as f64 % sps) / sps;
            let sample = if symbol_phase < 0.5 { 1.0 } else { -1.0 };
            if cr.process(sample).is_some() {
                decisions += 1;
            }
        }

        // Should get approximately 10 decisions (±1 due to startup)
        assert!(
            (9..=11).contains(&decisions),
            "Expected ~10 decisions, got {}",
            decisions
        );
    }

    #[test]
    fn test_omega_stays_clamped() {
        let sps = 1536.0;
        let mut cr = ClockRecovery::new(sps);

        // Feed garbage to try to destabilize omega
        for _ in 0..50000 {
            cr.process(1.0); // constant, creates large timing errors
        }

        assert!(cr.omega >= sps * 0.9);
        assert!(cr.omega <= sps * 1.1);
    }

    #[test]
    fn test_reset() {
        let sps = 1536.0;
        let mut cr = ClockRecovery::new(sps);

        // Process some samples
        for _ in 0..5000 {
            cr.process(0.5);
        }

        cr.reset();
        assert_eq!(cr.omega, sps);
        assert_eq!(cr.last_symbol, 0.0);
        assert_eq!(cr.sample_count, sps / 2.0);
    }
}
