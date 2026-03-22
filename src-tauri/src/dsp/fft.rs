//! FFT processing for waterfall display

use std::sync::Arc;
use rustfft::{Fft, FftPlanner, num_complex::Complex};

/// FFT processor for computing spectral data
pub struct FftProcessor {
    fft: Arc<dyn Fft<f32>>,
    fft_size: usize,
    window: Vec<f32>,
}

impl FftProcessor {
    /// Create a new FFT processor with the given size
    pub fn new(fft_size: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);

        // Generate Hanning window
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let x = std::f32::consts::PI * i as f32 / fft_size as f32;
                0.5 * (1.0 - (2.0 * x).cos())
            })
            .collect();

        Self {
            fft,
            fft_size,
            window,
        }
    }

    /// Compute FFT and return magnitude in dB
    /// Input should have at least `fft_size` samples
    pub fn compute(&mut self, samples: &[f32]) -> Vec<f32> {
        let fft = &self.fft;

        // Apply window and convert to complex
        let mut buffer: Vec<Complex<f32>> = samples
            .iter()
            .take(self.fft_size)
            .zip(self.window.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        // Pad if necessary
        buffer.resize(self.fft_size, Complex::new(0.0, 0.0));

        // Compute FFT in place
        fft.process(&mut buffer);

        // Convert to magnitude in dB (only positive frequencies)
        let half_size = self.fft_size / 2;
        buffer[..half_size]
            .iter()
            .map(|c| {
                let mag_squared = c.norm_sqr();
                // Convert to dB with floor to avoid -infinity
                10.0 * (mag_squared.max(1e-10)).log10()
            })
            .collect()
    }

    /// Get the FFT size
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_size_getter_returns_constructor_value() {
        let proc = FftProcessor::new(2048);
        assert_eq!(proc.fft_size(), 2048);
    }

    #[test]
    fn fft_size_getter_small() {
        let proc = FftProcessor::new(64);
        assert_eq!(proc.fft_size(), 64);
    }

    #[test]
    fn test_fft_pure_tone() {
        let mut processor = FftProcessor::new(1024);
        let sample_rate = 48000.0;
        let freq = 1000.0;

        // Generate a 1kHz sine wave
        let samples: Vec<f32> = (0..1024)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect();

        let spectrum = processor.compute(&samples);

        // Find the peak bin
        let peak_bin = spectrum
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        // Expected bin for 1kHz at 48kHz sample rate with 1024-point FFT
        // bin = freq * fft_size / sample_rate = 1000 * 1024 / 48000 ≈ 21.3
        let expected_bin = (freq * 1024.0 / sample_rate).round() as usize;

        assert!(
            (peak_bin as i32 - expected_bin as i32).abs() <= 1,
            "Peak at bin {} but expected near bin {}",
            peak_bin,
            expected_bin
        );
    }

    #[test]
    fn compute_repeated_calls_give_identical_results() {
        // Regression: before caching the FFT plan, plan_fft_forward() was called on every
        // compute() invocation. While not a correctness bug, repeated planning could return
        // different (or inconsistent) plan objects. Verify the cached plan produces
        // bit-identical output across multiple calls on the same input.
        let mut processor = FftProcessor::new(1024);
        let samples: Vec<f32> = (0..1024)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin())
            .collect();

        let first = processor.compute(&samples);
        let second = processor.compute(&samples);

        assert_eq!(
            first, second,
            "repeated compute() calls must return identical results"
        );
    }
}
