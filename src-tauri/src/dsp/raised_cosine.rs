//! Raised cosine pulse shaping for TX

use std::f32::consts::PI;

/// Generate a raised cosine envelope for smooth phase transitions
pub struct RaisedCosineShaper {
    samples_per_symbol: usize,
}

impl RaisedCosineShaper {
    pub fn new(samples_per_symbol: usize) -> Self {
        Self { samples_per_symbol }
    }

    /// Generate envelope values for one symbol period
    /// Returns a vector of envelope multipliers (0.0 to 1.0)
    pub fn generate_envelope(&self, phase_change: bool) -> Vec<f32> {
        let n = self.samples_per_symbol;
        let mut envelope = vec![1.0; n];

        if phase_change {
            // Apply raised cosine ramp down then up
            for i in 0..n {
                let t = i as f32 / n as f32;
                // Cosine envelope: cos^2 shape centered at symbol midpoint
                envelope[i] = (PI * t).cos().abs();
            }
        }

        envelope
    }

    /// Get samples per symbol
    pub fn samples_per_symbol(&self) -> usize {
        self.samples_per_symbol
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_phase_change_envelope() {
        let shaper = RaisedCosineShaper::new(1536);
        let envelope = shaper.generate_envelope(false);

        assert_eq!(envelope.len(), 1536);
        assert!(envelope.iter().all(|&e| e == 1.0));
    }

    #[test]
    fn test_phase_change_envelope() {
        let shaper = RaisedCosineShaper::new(1536);
        let envelope = shaper.generate_envelope(true);

        assert_eq!(envelope.len(), 1536);
        // Should start at 1.0, dip to 0.0 at center, back to 1.0
        assert!((envelope[0] - 1.0).abs() < 0.01);
        assert!(envelope[768] < 0.1); // Near zero at center
    }

    #[test]
    fn test_samples_per_symbol_getter() {
        let shaper = RaisedCosineShaper::new(1536);
        assert_eq!(shaper.samples_per_symbol(), 1536);
    }

    #[test]
    fn test_samples_per_symbol_getter_small() {
        let shaper = RaisedCosineShaper::new(1);
        assert_eq!(shaper.samples_per_symbol(), 1);
    }

    #[test]
    fn test_single_sample_no_phase_change() {
        let shaper = RaisedCosineShaper::new(1);
        let envelope = shaper.generate_envelope(false);
        assert_eq!(envelope.len(), 1);
        assert_eq!(envelope[0], 1.0);
    }

    #[test]
    fn test_single_sample_phase_change() {
        let shaper = RaisedCosineShaper::new(1);
        let envelope = shaper.generate_envelope(true);
        assert_eq!(envelope.len(), 1);
        // At t=0/1=0.0: |cos(pi * 0)| = |cos(0)| = 1.0
        assert!((envelope[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_large_samples_per_symbol() {
        let shaper = RaisedCosineShaper::new(48000);
        let envelope = shaper.generate_envelope(true);
        assert_eq!(envelope.len(), 48000);
        // All values should be in [0, 1]
        assert!(envelope.iter().all(|&v| v >= 0.0 && v <= 1.0));
    }

    #[test]
    fn test_envelope_values_non_negative() {
        let shaper = RaisedCosineShaper::new(1536);
        let envelope = shaper.generate_envelope(true);
        assert!(
            envelope.iter().all(|&v| v >= 0.0),
            "All envelope values should be non-negative (abs() applied)"
        );
    }
}
