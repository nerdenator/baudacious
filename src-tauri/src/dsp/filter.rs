//! FIR filter implementation

/// FIR filter for bandpass/lowpass filtering
pub struct FirFilter {
    coefficients: Vec<f32>,
    delay_line: Vec<f32>,
    position: usize,
}

impl FirFilter {
    /// Create a new FIR filter with the given coefficients
    pub fn new(coefficients: Vec<f32>) -> Self {
        let len = coefficients.len();
        Self {
            coefficients,
            delay_line: vec![0.0; len],
            position: 0,
        }
    }

    /// Create a simple lowpass filter using windowed sinc
    pub fn lowpass(cutoff_freq: f32, sample_rate: f32, num_taps: usize) -> Self {
        let normalized_cutoff = cutoff_freq / sample_rate;
        let mut coefficients = vec![0.0; num_taps];
        let middle = num_taps / 2;

        for i in 0..num_taps {
            let n = i as f32 - middle as f32;
            if n == 0.0 {
                coefficients[i] = 2.0 * normalized_cutoff;
            } else {
                coefficients[i] = (2.0 * std::f32::consts::PI * normalized_cutoff * n).sin()
                    / (std::f32::consts::PI * n);
            }

            // Apply Hanning window
            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / num_taps as f32).cos());
            coefficients[i] *= window;
        }

        // Normalize
        let sum: f32 = coefficients.iter().sum();
        for c in &mut coefficients {
            *c /= sum;
        }

        Self::new(coefficients)
    }

    /// Create a bandpass filter by frequency-shifting a lowpass prototype
    ///
    /// Like taking a lowpass filter and "sliding" it up to center_freq on the
    /// spectrum — multiply the lowpass coefficients by cos(2π·f_c·n) to shift.
    pub fn bandpass(center_freq: f32, bandwidth: f32, sample_rate: f32, num_taps: usize) -> Self {
        // Start with a lowpass prototype at bandwidth/2
        let normalized_cutoff = (bandwidth / 2.0) / sample_rate;
        let middle = num_taps / 2;
        let mut coefficients = vec![0.0; num_taps];

        for i in 0..num_taps {
            let n = i as f32 - middle as f32;

            // Lowpass sinc kernel
            let lp = if n == 0.0 {
                2.0 * normalized_cutoff
            } else {
                (2.0 * std::f32::consts::PI * normalized_cutoff * n).sin()
                    / (std::f32::consts::PI * n)
            };

            // Hanning window
            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / num_taps as f32).cos());

            // Shift to center_freq by multiplying by cosine
            let shift = (2.0 * std::f32::consts::PI * center_freq * n / sample_rate).cos();

            coefficients[i] = lp * window * shift;
        }

        // Normalize for unity passband gain: scale so that a tone at center_freq
        // passes through with amplitude ~1.0
        let mut response_real = 0.0f32;
        let mut response_imag = 0.0f32;
        for (i, &c) in coefficients.iter().enumerate() {
            let n = i as f32;
            let angle = 2.0 * std::f32::consts::PI * center_freq * n / sample_rate;
            response_real += c * angle.cos();
            response_imag += c * angle.sin();
        }
        let response_mag = (response_real * response_real + response_imag * response_imag).sqrt();

        if response_mag > 1e-10 {
            for c in &mut coefficients {
                *c /= response_mag;
            }
        }

        Self::new(coefficients)
    }

    /// Process a single sample through the filter
    pub fn process(&mut self, sample: f32) -> f32 {
        self.delay_line[self.position] = sample;

        let mut output = 0.0;
        let len = self.coefficients.len();

        for i in 0..len {
            let delay_idx = (self.position + len - i) % len;
            output += self.coefficients[i] * self.delay_line[delay_idx];
        }

        self.position = (self.position + 1) % len;
        output
    }

    /// Reset the filter state
    pub fn reset(&mut self) {
        self.delay_line.fill(0.0);
        self.position = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lowpass_passes_dc() {
        let mut filter = FirFilter::lowpass(1000.0, 48000.0, 63);

        // Feed DC (constant 1.0) through the filter — should converge to ~1.0
        let mut output = 0.0;
        for _ in 0..200 {
            output = filter.process(1.0);
        }

        assert!(
            (output - 1.0).abs() < 0.01,
            "DC signal should pass through lowpass unchanged, got {}",
            output
        );
    }

    #[test]
    fn test_lowpass_attenuates_high_frequency() {
        // Lowpass at 100 Hz, feed a 10 kHz sine — should be heavily attenuated
        let mut filter = FirFilter::lowpass(100.0, 48000.0, 63);

        // Let the filter settle
        let freq = 10000.0;
        let mut max_output = 0.0f32;
        for i in 0..1000 {
            let sample = (2.0 * std::f32::consts::PI * freq * i as f32 / 48000.0).sin();
            let out = filter.process(sample);
            if i > 100 {
                // Skip transient
                max_output = max_output.max(out.abs());
            }
        }

        assert!(
            max_output < 0.05,
            "10 kHz signal should be attenuated by lowpass at 100 Hz, got {}",
            max_output
        );
    }

    #[test]
    fn test_lowpass_coefficients_normalized() {
        let filter = FirFilter::lowpass(1000.0, 48000.0, 63);
        // Coefficients should sum to ~1.0 (unity DC gain)
        let sum: f32 = filter.coefficients.iter().sum();
        assert!(
            (sum - 1.0).abs() < 0.01,
            "Coefficients should sum to ~1.0, got {}",
            sum
        );
    }

    #[test]
    fn test_bandpass_passes_center_frequency() {
        let mut filter = FirFilter::bandpass(1000.0, 100.0, 48000.0, 127);

        // Feed a 1000 Hz sine through — should pass
        let mut max_output = 0.0f32;
        for i in 0..5000 {
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin();
            let out = filter.process(sample);
            if i > 500 {
                max_output = max_output.max(out.abs());
            }
        }

        assert!(
            max_output > 0.5,
            "1000 Hz signal should pass through bandpass centered at 1000 Hz, got {}",
            max_output
        );
    }

    #[test]
    fn test_bandpass_rejects_far_frequency() {
        let mut filter = FirFilter::bandpass(1000.0, 100.0, 48000.0, 127);

        // Feed a 3000 Hz sine — should be attenuated
        let mut max_output = 0.0f32;
        for i in 0..5000 {
            let sample = (2.0 * std::f32::consts::PI * 3000.0 * i as f32 / 48000.0).sin();
            let out = filter.process(sample);
            if i > 500 {
                max_output = max_output.max(out.abs());
            }
        }

        assert!(
            max_output < 0.1,
            "3000 Hz signal should be rejected by bandpass at 1000 Hz, got {}",
            max_output
        );
    }

    #[test]
    fn test_reset_clears_state() {
        let mut filter = FirFilter::lowpass(1000.0, 48000.0, 63);

        // Feed some samples
        for _ in 0..100 {
            filter.process(1.0);
        }

        filter.reset();

        // After reset, processing 0.0 should give 0.0
        let out = filter.process(0.0);
        assert_eq!(out, 0.0);
    }

    #[test]
    fn test_lowpass_low_cutoff_attenuates_above_passband() {
        // Cutoff at 1000 Hz — 5000 Hz input should be strongly attenuated.
        // A 63-tap FIR at 48 kHz has a minimum practical transition band of
        // ~762 Hz, so 1000 Hz cutoff vs 5000 Hz signal is well within range.
        let mut filter = FirFilter::lowpass(1000.0, 48000.0, 63);
        let mut max_output = 0.0f32;
        for i in 0..500 {
            let sample = (2.0 * std::f32::consts::PI * 5000.0 * i as f32 / 48000.0).sin();
            let out = filter.process(sample);
            if i > 100 {
                max_output = max_output.max(out.abs());
            }
        }
        assert!(
            max_output < 0.1,
            "5000 Hz should be attenuated by 1000 Hz lowpass, got {}",
            max_output
        );
    }

    #[test]
    fn test_lowpass_near_nyquist_passes_dc() {
        // Cutoff near Nyquist (20kHz at 48kHz) — DC should still pass
        let mut filter = FirFilter::lowpass(20000.0, 48000.0, 63);
        let mut output = 0.0;
        for _ in 0..200 {
            output = filter.process(1.0);
        }
        assert!(
            (output - 1.0).abs() < 0.05,
            "DC should pass near-Nyquist lowpass, got {}",
            output
        );
    }

    #[test]
    fn test_bandpass_very_wide_bandwidth_passes_center() {
        // Wide bandwidth (8000 Hz) bandpass centered at 4000 Hz
        let mut filter = FirFilter::bandpass(4000.0, 8000.0, 48000.0, 127);
        let mut max_output = 0.0f32;
        for i in 0..5000 {
            let sample = (2.0 * std::f32::consts::PI * 4000.0 * i as f32 / 48000.0).sin();
            let out = filter.process(sample);
            if i > 500 {
                max_output = max_output.max(out.abs());
            }
        }
        assert!(
            max_output > 0.3,
            "Center frequency should pass wide bandpass, got {}",
            max_output
        );
    }

    #[test]
    fn test_bandpass_rejects_dc() {
        // Bandpass centered at 2000 Hz should reject DC
        let mut filter = FirFilter::bandpass(2000.0, 200.0, 48000.0, 127);
        let mut output = 0.0f32;
        for _ in 0..500 {
            output = filter.process(1.0);
        }
        assert!(
            output.abs() < 0.1,
            "DC should be rejected by bandpass at 2000 Hz, got {}",
            output
        );
    }
}
