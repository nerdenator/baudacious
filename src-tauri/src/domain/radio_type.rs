//! Supported radio models and their band capabilities
//!
//! Pure data: which US amateur bands each radio can tune, and which `BS;`
//! (band-select) code, if any, selects each band. Adapters consult this
//! instead of keeping private copies of the tables.

use std::str::FromStr;

use super::error::Psk31Error;

/// Radio models the app can drive over CAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioType {
    Ft991a,
    Ft891,
}

/// How a radio handles a given band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Support {
    /// The radio cannot tune this band at all.
    No,
    /// Tunable, selected with `BS{code};`.
    WithBs(u8),
    /// Tunable via `FA;` only — the radio has no band-select code for it.
    FreqOnly,
}

struct Band {
    low_hz: u64,
    high_hz: u64,
    ft991a: Support,
    ft891: Support,
}

use Support::{FreqOnly, No, WithBs};

/// US amateur bands (FCC Part 97) with per-radio support.
///
/// FT-991A codes: 0=160m … 10=6m, 12=2m, 13=70cm.
/// FT-891 codes (CAT manual, `BS`): same 0–10 except `02` is unused, so 60m is
/// frequency-only; no 2m/70cm.
const BANDS: &[Band] = &[
    Band { low_hz: 1_800_000,   high_hz: 2_000_000,   ft991a: WithBs(0),  ft891: WithBs(0)  }, // 160m
    Band { low_hz: 3_500_000,   high_hz: 4_000_000,   ft991a: WithBs(1),  ft891: WithBs(1)  }, // 80m
    Band { low_hz: 5_332_000,   high_hz: 5_405_000,   ft991a: WithBs(2),  ft891: FreqOnly   }, // 60m
    Band { low_hz: 7_000_000,   high_hz: 7_300_000,   ft991a: WithBs(3),  ft891: WithBs(3)  }, // 40m
    Band { low_hz: 10_100_000,  high_hz: 10_150_000,  ft991a: WithBs(4),  ft891: WithBs(4)  }, // 30m
    Band { low_hz: 14_000_000,  high_hz: 14_350_000,  ft991a: WithBs(5),  ft891: WithBs(5)  }, // 20m
    Band { low_hz: 18_068_000,  high_hz: 18_168_000,  ft991a: WithBs(6),  ft891: WithBs(6)  }, // 17m
    Band { low_hz: 21_000_000,  high_hz: 21_450_000,  ft991a: WithBs(7),  ft891: WithBs(7)  }, // 15m
    Band { low_hz: 24_890_000,  high_hz: 24_990_000,  ft991a: WithBs(8),  ft891: WithBs(8)  }, // 12m
    Band { low_hz: 28_000_000,  high_hz: 29_700_000,  ft991a: WithBs(9),  ft891: WithBs(9)  }, // 10m
    Band { low_hz: 50_000_000,  high_hz: 54_000_000,  ft991a: WithBs(10), ft891: WithBs(10) }, // 6m
    Band { low_hz: 144_000_000, high_hz: 148_000_000, ft991a: WithBs(12), ft891: No         }, // 2m
    Band { low_hz: 420_000_000, high_hz: 450_000_000, ft991a: WithBs(13), ft891: No         }, // 70cm
];

impl RadioType {
    /// Name used in saved configuration and the settings dialog.
    pub fn as_str(&self) -> &'static str {
        match self {
            RadioType::Ft991a => "FT-991A",
            RadioType::Ft891 => "FT-891",
        }
    }

    /// Suggested CAT baud rate. The FT-891's rate is a radio menu setting
    /// (05-06) and must match; 9600 is the reference book's listed default.
    pub fn default_baud(&self) -> u32 {
        match self {
            RadioType::Ft991a => 38400,
            RadioType::Ft891 => 9600,
        }
    }

    /// Inclusive TX power range in watts accepted by `PC;` on this radio.
    pub fn tx_power_range(&self) -> (u32, u32) {
        match self {
            RadioType::Ft991a => (0, 100),
            RadioType::Ft891 => (5, 100),
        }
    }

    fn support(&self, band: &Band) -> Support {
        match self {
            RadioType::Ft991a => band.ft991a,
            RadioType::Ft891 => band.ft891,
        }
    }

    fn band_for(&self, hz: u64) -> Option<Support> {
        BANDS
            .iter()
            .find(|b| hz >= b.low_hz && hz <= b.high_hz)
            .map(|b| self.support(b))
            .filter(|s| *s != No)
    }

    /// True if `hz` is inside a US amateur band this radio can tune.
    pub fn supports_frequency(&self, hz: u64) -> bool {
        self.band_for(hz).is_some()
    }

    /// `BS;` code for the band containing `hz`.
    ///
    /// `None` if the frequency is outside this radio's bands, or the band has
    /// no band-select code (FT-891 60m — set the frequency directly).
    pub fn band_select_code(&self, hz: u64) -> Option<u8> {
        match self.band_for(hz) {
            Some(WithBs(code)) => Some(code),
            _ => None,
        }
    }
}

impl FromStr for RadioType {
    type Err = Psk31Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "FT-991A" => Ok(RadioType::Ft991a),
            "FT-891" => Ok(RadioType::Ft891),
            other => Err(Psk31Error::Config(format!("Unsupported radio type: '{other}'"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_types() {
        assert_eq!("FT-991A".parse::<RadioType>().unwrap(), RadioType::Ft991a);
        assert_eq!("FT-891".parse::<RadioType>().unwrap(), RadioType::Ft891);
    }

    #[test]
    fn parse_unknown_type_is_error() {
        assert!("FT-817".parse::<RadioType>().is_err());
        assert!("".parse::<RadioType>().is_err());
        assert!("ft-891".parse::<RadioType>().is_err(), "match is case-sensitive");
    }

    #[test]
    fn as_str_round_trips() {
        for rt in [RadioType::Ft991a, RadioType::Ft891] {
            assert_eq!(rt.as_str().parse::<RadioType>().unwrap(), rt);
        }
    }

    #[test]
    fn default_bauds() {
        assert_eq!(RadioType::Ft991a.default_baud(), 38400);
        assert_eq!(RadioType::Ft891.default_baud(), 9600);
    }

    #[test]
    fn tx_power_ranges() {
        assert_eq!(RadioType::Ft991a.tx_power_range(), (0, 100));
        assert_eq!(RadioType::Ft891.tx_power_range(), (5, 100));
    }

    #[test]
    fn ft891_rejects_vhf_uhf() {
        for hz in [144_000_000, 146_520_000, 148_000_000, 420_000_000, 432_100_000, 450_000_000] {
            assert!(!RadioType::Ft891.supports_frequency(hz), "{hz}");
            assert_eq!(RadioType::Ft891.band_select_code(hz), None, "{hz}");
        }
    }

    #[test]
    fn ft991a_accepts_vhf_uhf() {
        assert!(RadioType::Ft991a.supports_frequency(144_144_000));
        assert!(RadioType::Ft991a.supports_frequency(432_100_000));
        assert_eq!(RadioType::Ft991a.band_select_code(144_144_000), Some(12));
        assert_eq!(RadioType::Ft991a.band_select_code(432_100_000), Some(13));
    }

    #[test]
    fn sixty_meters_ft891_is_frequency_only() {
        assert!(RadioType::Ft891.supports_frequency(5_358_500));
        assert_eq!(RadioType::Ft891.band_select_code(5_358_500), None);
        assert_eq!(RadioType::Ft991a.band_select_code(5_358_500), Some(2));
    }

    #[test]
    fn hf_and_6m_codes_match_on_both_radios() {
        let cases: &[(u64, u8)] = &[
            (1_900_000, 0), (3_600_000, 1), (7_035_000, 3), (10_120_000, 4),
            (14_070_000, 5), (18_100_000, 6), (21_070_000, 7), (24_920_000, 8),
            (28_120_000, 9), (50_313_000, 10),
        ];
        for &(hz, code) in cases {
            assert_eq!(RadioType::Ft991a.band_select_code(hz), Some(code), "991A {hz}");
            assert_eq!(RadioType::Ft891.band_select_code(hz), Some(code), "891 {hz}");
        }
    }

    #[test]
    fn band_edges_inclusive_and_gaps_rejected() {
        for rt in [RadioType::Ft991a, RadioType::Ft891] {
            assert!(rt.supports_frequency(14_000_000));
            assert!(rt.supports_frequency(14_350_000));
            assert!(!rt.supports_frequency(13_999_999));
            assert!(!rt.supports_frequency(14_350_001));
            assert!(!rt.supports_frequency(11_000_000), "gap between 30m and 20m");
            assert!(!rt.supports_frequency(0));
        }
    }
}
