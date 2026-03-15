# Plan: Move Sideband Convention into Domain Layer

## Status

- [x] S1 — Move `data_mode_for_frequency` to `domain/`
- [x] S2 — Update call sites

---

## Problem

`data_mode_for_frequency` encodes FCC Part 97 sideband conventions (LSB below 10 MHz, USB
above, 60m exception). This is pure domain knowledge — no I/O, no hardware dependency. It
currently lives in `commands/tx.rs`, which is the adapter layer. Any other future command
that needs to respect this rule would have to import from `commands/`, which inverts the
dependency direction.

`ensure_data_mode` is application-layer orchestration (read freq → compute target → correct
radio) and belongs in `commands/`. It stays where it is; only its dependency on
`data_mode_for_frequency` changes.

---

## Target Architecture

```
domain/frequency.rs  (or domain/band_plan.rs)
  └── pub fn data_mode_for_frequency(hz: f64) -> &'static str   ← moves here

commands/tx.rs
  └── ensure_data_mode(&mut dyn RadioControl)                    ← stays, imports from domain
```

The `data_mode_for_frequency` function has no I/O and no dependencies — it is a pure
mapping from frequency to mode string. Domain is the correct layer.

---

## S1 — Add `data_mode_for_frequency` to the domain layer

**File to add to:** `src-tauri/src/domain/frequency.rs`

Append the function (with its existing doc comment and 60m exception) to the bottom of
`frequency.rs`, where the `Frequency` type already lives.

```rust
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
```

---

## S2 — Update `commands/tx.rs`

1. Delete the `data_mode_for_frequency` function and its doc comment (lines 27–44).
2. Add an import: `use crate::domain::frequency::data_mode_for_frequency;`
   (or `use crate::domain::data_mode_for_frequency;` depending on re-export — check
   `domain/mod.rs` for whether `frequency` items are re-exported at the domain level).
3. `ensure_data_mode` continues to call `data_mode_for_frequency` unchanged.

**No other files change.** `ensure_data_mode` stays in `commands/tx.rs` — it is
application-layer orchestration of the radio port, not domain logic.

---

## Verification

```bash
# in src-tauri/
cargo test    # existing + new sideband unit tests must pass
cargo check   # no broken imports
```

---

## Commit Strategy

| Commit | Change |
|--------|--------|
| 1 | S1 + S2: move `data_mode_for_frequency` to `domain/frequency.rs`, add unit tests, update import in `commands/tx.rs` |

Single commit — the two files change together and the diff tells a complete story.

---

## Sequencing

Execute after `PLAN_PIPELINE_REFACTOR.md` is complete.
