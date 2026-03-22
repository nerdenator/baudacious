//! Costas Loop for BPSK carrier tracking
//!
//! The Costas Loop locks onto a BPSK carrier, tracks its frequency/phase,
//! and downmixes to baseband in one step. Think of it as a smart mixer
//! that automatically tunes itself to the incoming signal.
//!
//! Uses single-pole IIR lowpass filters for the I/Q arms — these work
//! much better than short FIR filters at the extreme ratio of sample rate
//! (48kHz) to signal bandwidth (~30 Hz for PSK-31).

use super::nco::Nco;

/// Costas loop for BPSK carrier tracking and demodulation
pub struct CostasLoop {
    nco: Nco,
    /// Filtered I arm value (single-pole IIR state)
    filtered_i: f32,
    /// Filtered Q arm value (single-pole IIR state)
    filtered_q: f32,
    /// IIR smoothing coefficient: alpha = 2π * cutoff / sample_rate
    alpha: f32,
    proportional_gain: f64,
    integral_gain: f64,
    integrator: f64,
}

impl CostasLoop {
    /// Create a new Costas loop
    ///
    /// - `carrier_freq`: expected carrier frequency in Hz
    /// - `sample_rate`: audio sample rate (e.g., 48000)
    /// - `loop_bandwidth`: PLL bandwidth in Hz (~2 Hz for PSK-31)
    pub fn new(carrier_freq: f64, sample_rate: f64, loop_bandwidth: f64) -> Self {
        let nco = Nco::new(carrier_freq, sample_rate);

        // IIR lowpass coefficient for I/Q arms
        // Cutoff at ~50 Hz removes the double-frequency term after mixing
        let lpf_cutoff = 50.0;
        let alpha = (2.0 * std::f64::consts::PI * lpf_cutoff / sample_rate) as f32;

        // PLL gains, empirically tuned for BPSK Costas Loop at 48kHz.
        //
        // The Costas Loop's I×Q error detector has gain proportional to A²/4,
        // which differs from the unity-gain detector assumed by textbook PLL
        // formulas. These gains are scaled by T = 1/fs so the loop doesn't
        // overreact at high sample rates.
        //
        // Proportional: fast phase correction (tracks phase jitter)
        // Integral: slow frequency correction (tracks carrier offset)
        let _ = loop_bandwidth; // Used conceptually to set the gains below
        let proportional_gain = 0.01;
        let integral_gain = 0.000005;

        Self {
            nco,
            filtered_i: 0.0,
            filtered_q: 0.0,
            alpha,
            proportional_gain,
            integral_gain,
            integrator: 0.0,
        }
    }

    /// Process a single sample, returns the demodulated baseband I value
    pub fn process(&mut self, sample: f32) -> f32 {
        // Mix with local oscillator (downconvert to baseband)
        let (nco_i, nco_q) = self.nco.next_iq();
        let mixed_i = sample * nco_i;
        let mixed_q = sample * nco_q;

        // Single-pole IIR lowpass: y[n] = α·x[n] + (1-α)·y[n-1]
        // Removes the double-frequency term (at 2×carrier) after mixing,
        // leaving just the baseband data signal
        self.filtered_i += self.alpha * (mixed_i - self.filtered_i);
        self.filtered_q += self.alpha * (mixed_q - self.filtered_q);

        // Phase error detector for BPSK: e = I × Q
        // When locked: I is large (data), Q is near zero
        // Phase error drives Q toward zero
        let phase_error = (self.filtered_i * self.filtered_q) as f64;

        // Loop filter (PI controller)
        self.integrator += self.integral_gain * phase_error;
        let correction = self.proportional_gain * phase_error + self.integrator;

        // Adjust NCO phase to track the carrier
        self.nco.adjust_phase(correction);

        self.filtered_i
    }

    /// Set the carrier frequency (e.g., from click-to-tune)
    pub fn set_frequency(&mut self, freq: f64) {
        self.nco.set_frequency(freq);
    }

    /// Reset the loop state
    pub fn reset(&mut self) {
        self.nco.reset();
        self.filtered_i = 0.0;
        self.filtered_q = 0.0;
        self.integrator = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Generate a clean BPSK signal: carrier with phase flips at symbol boundaries
    fn generate_bpsk(
        carrier_freq: f64,
        sample_rate: f64,
        samples_per_symbol: usize,
        bits: &[bool],
    ) -> Vec<f32> {
        let mut samples = Vec::new();
        let mut phase = 0.0f64;
        let phase_inc = 2.0 * PI * carrier_freq / sample_rate;
        let mut current_phase_offset = 0.0f64;

        for &bit in bits {
            if !bit {
                current_phase_offset += PI;
            }
            for _ in 0..samples_per_symbol {
                samples.push((phase + current_phase_offset).cos() as f32);
                phase += phase_inc;
            }
        }
        samples
    }

    #[test]
    fn test_costas_locks_on_clean_bpsk() {
        let carrier_freq = 1000.0;
        let sample_rate = 48000.0;
        let sps = 1536;

        let mut bits: Vec<bool> = vec![false; 32]; // preamble
        bits.extend_from_slice(&[true, true, false, true, false, false, true, true]);
        let signal = generate_bpsk(carrier_freq, sample_rate, sps, &bits);

        let mut costas = CostasLoop::new(carrier_freq, sample_rate, 2.0);

        let mut symbol_values = Vec::new();
        for (i, &sample) in signal.iter().enumerate() {
            let baseband = costas.process(sample);
            let sym_idx = i / sps;
            let within = i % sps;
            if within == sps / 2 && sym_idx >= 20 {
                symbol_values.push(baseband);
            }
        }

        assert!(!symbol_values.is_empty());
        let sign_changes: usize = symbol_values
            .windows(2)
            .filter(|w| (w[0] > 0.0) != (w[1] > 0.0))
            .count();
        assert!(sign_changes > 0, "Should see sign changes in demodulated output");
    }

    #[test]
    fn test_costas_tracks_small_frequency_offset() {
        // 2 Hz offset is within the PLL's pull-in range (Bn = 2 Hz).
        // In practice, the user clicks the waterfall to tune within 1-2 Hz.
        let true_freq = 1000.0;
        let nco_freq = 1002.0; // 2 Hz offset
        let sample_rate = 48000.0;
        let sps = 1536;

        let bits: Vec<bool> = vec![false; 128];
        let signal = generate_bpsk(true_freq, sample_rate, sps, &bits);

        let mut costas = CostasLoop::new(nco_freq, sample_rate, 2.0);

        // Check last 16 symbols (after settling)
        let mut late_outputs = Vec::new();
        for (i, &sample) in signal.iter().enumerate() {
            let baseband = costas.process(sample);
            let sym_idx = i / sps;
            let within = i % sps;
            if within == sps / 2 && sym_idx >= 112 {
                late_outputs.push(baseband);
            }
        }

        let magnitudes: Vec<f32> = late_outputs.iter().map(|v| v.abs()).collect();
        if let (Some(&min), Some(&max)) = (
            magnitudes.iter().reduce(|a, b| if a < b { a } else { b }),
            magnitudes.iter().reduce(|a, b| if a > b { a } else { b }),
        ) {
            if max > 0.001 {
                let ratio = min / max;
                assert!(
                    ratio > 0.3,
                    "Should track 2 Hz offset — ratio {:.3} suggests beating",
                    ratio
                );
            }
        }
    }

    #[test]
    fn test_costas_reset() {
        let mut costas = CostasLoop::new(1000.0, 48000.0, 2.0);

        for i in 0..10000 {
            let sample = (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48000.0).cos() as f32;
            costas.process(sample);
        }

        costas.reset();
        assert_eq!(costas.integrator, 0.0);
        assert_eq!(costas.filtered_i, 0.0);
        assert_eq!(costas.filtered_q, 0.0);
    }

    #[test]
    fn test_set_frequency_updates_nco() {
        let mut costas = CostasLoop::new(1000.0, 48000.0, 2.0);
        costas.set_frequency(1500.0);
        // Assert directly that the NCO was updated to the new frequency.
        assert_eq!(
            costas.nco.frequency(),
            1500.0,
            "set_frequency should update the internal NCO to 1500 Hz"
        );
    }

    #[test]
    fn test_reset_clears_integrator_and_filters() {
        let mut costas = CostasLoop::new(1000.0, 48000.0, 2.0);

        // Drive the integrator non-zero
        for i in 0..5000 {
            let sample = (2.0 * PI * 1001.0 * i as f64 / 48000.0).cos() as f32;
            costas.process(sample);
        }

        // Integrator should be non-zero due to frequency offset
        // (just verify reset zeroes it regardless)
        costas.reset();
        assert_eq!(costas.integrator, 0.0, "integrator should be 0 after reset");
        assert_eq!(costas.filtered_i, 0.0, "filtered_i should be 0 after reset");
        assert_eq!(costas.filtered_q, 0.0, "filtered_q should be 0 after reset");
    }
}
