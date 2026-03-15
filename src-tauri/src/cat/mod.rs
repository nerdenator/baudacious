//! CAT (Computer Aided Transceiver) command layer for the FT-991A.
//!
//! This module separates the three concerns of CAT communication:
//! - `encode`: translate CatCommand → wire string (pure, no I/O)
//! - `decode`: translate wire string → CatResponse (pure, no I/O)
//! - `session`: own the serial port, drive timing and I/O
//!
//! The encode/decode functions are pure so they can be tested without
//! any mock serial port.

pub mod decode;
pub mod encode;
pub mod session;

pub use decode::decode;
pub use encode::encode;
pub use session::CatSession;

use crate::domain::RadioStatus;

/// Single source of truth for FT-991A mode code ↔ name mapping.
/// Each entry is (CAT code, human-readable name).
pub const MODE_TABLE: &[(&str, &str)] = &[
    ("1", "LSB"),
    ("2", "USB"),
    ("3", "CW"),
    ("4", "FM"),
    ("5", "AM"),
    ("6", "RTTY-LSB"),
    ("7", "CW-R"),
    ("8", "DATA-LSB"),
    ("9", "RTTY-USB"),
    ("A", "DATA-FM"),
    ("B", "FM-N"),
    ("C", "DATA-USB"),
    ("D", "AM-N"),
    ("E", "C4FM"),
];

/// High-level CAT commands understood by the FT-991A.
#[derive(Debug, PartialEq, Clone)]
pub enum CatCommand {
    // VFO-A frequency
    GetFrequencyA,
    SetFrequencyA(u64),
    // Operating mode
    GetMode,
    /// Mode name e.g. "DATA-USB"
    SetMode(String),
    // PTT control
    PttOff,
    PttOn,
    // TX power
    GetTxPower,
    /// Watts, 0–100
    SetTxPower(u32),
    // S-meter
    GetSignalStrength,
    // Comprehensive status (IF; command)
    GetStatus,
    /// Select band group (BS; command).  Code 0–10=HF/6m, 12=2m, 13=70cm.
    /// The FT-991A executes BS; silently — no ack is returned.
    BandSelect(u8),
}

/// Parsed responses from the FT-991A.
#[derive(Debug, PartialEq)]
pub enum CatResponse {
    FrequencyHz(u64),
    /// Human-readable mode name e.g. "DATA-USB"
    Mode(String),
    /// TX power in watts
    TxPower(u32),
    /// S-meter level, normalised 0.0–1.0 from the 0–30 SM0 scale
    SignalStrength(f32),
    /// Full radio status from the IF; command
    Status(RadioStatus),
    /// Command accepted; radio returned just ";"
    Ack,
}
