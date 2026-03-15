//! Frequency-related domain helpers

/// Determine the correct PSK-31 DATA mode for a given radio frequency.
///
/// By HF convention:
/// - Below 10 MHz (160m, 80m, 40m): lower sideband → DATA-LSB
/// - 10 MHz and above (30m through 10m): upper sideband → DATA-USB
///
/// Exception: 60m (5.332–5.405 MHz) is USB-only per FCC Part 97.307(f)(11).
pub fn data_mode_for_frequency(hz: f64) -> &'static str {
    // 60m: FCC Part 97.307(f)(11) mandates USB regardless of the below-10-MHz convention
    if (5_332_000.0..=5_405_000.0).contains(&hz) {
        return "DATA-USB";
    }
    if hz < 10_000_000.0 {
        "DATA-LSB"
    } else {
        "DATA-USB"
    }
}

#[cfg(test)]
mod sideband_tests {
    use super::*;

    #[test]
    fn below_10mhz_is_lsb() {
        assert_eq!(data_mode_for_frequency(7_074_000.0), "DATA-LSB"); // 40m
        assert_eq!(data_mode_for_frequency(3_580_000.0), "DATA-LSB"); // 80m
    }

    #[test]
    fn above_10mhz_is_usb() {
        assert_eq!(data_mode_for_frequency(14_070_000.0), "DATA-USB"); // 20m
        assert_eq!(data_mode_for_frequency(21_080_000.0), "DATA-USB"); // 15m
    }

    #[test]
    fn sixty_meters_is_usb_exception() {
        assert_eq!(data_mode_for_frequency(5_357_000.0), "DATA-USB"); // 60m calling freq
        assert_eq!(data_mode_for_frequency(5_332_000.0), "DATA-USB"); // lower edge
        assert_eq!(data_mode_for_frequency(5_405_000.0), "DATA-USB"); // upper edge
    }

    #[test]
    fn boundary_at_10mhz_is_usb() {
        assert_eq!(data_mode_for_frequency(10_000_000.0), "DATA-USB"); // 30m lower edge
    }
}
