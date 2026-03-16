# PLAN_COVERAGE.md — Test Coverage Improvement Plan

**Project**: Baudacious (PSK-31 desktop ham radio client)
**Date**: 2026-03-15
**Goal**: Rust backend ~80%+ line coverage; frontend branch coverage ~75%+

---

## Current State

| Layer | Lines | Branches | Functions |
|---|---|---|---|
| Rust (`cargo llvm-cov --lib`) | 64% | 63% | 64.5% |
| TypeScript (Istanbul / Playwright) | 76.7% | 59.4% | 68.1% |

---

## What to Skip and Why

These modules are 0% covered and must stay that way — testing them requires infrastructure investment far beyond the return:

| Module | Why Skip |
|---|---|
| `commands/audio.rs` | Requires real `cpal` hardware and a live Tauri `AppHandle` |
| `commands/serial.rs` | Requires a real serial port or full Tauri test runtime |
| `commands/tx.rs` | Spawns real OS threads with Tauri handles |
| `commands/status.rs` | Purely a Tauri wrapper around `AppState` |
| `adapters/cpal_audio.rs` | Requires physical audio hardware |
| `lib.rs` | Tauri application wiring — not unit testable |
| `menu.rs` | Requires a native macOS menu context |
| `ports/serial.rs` | Pure trait definition — no logic |

Do not add a `tauri::test` harness just for these. They are verified by the E2E tests at the integration level.

---

## Prioritized Test Plan

### Priority 1 — `adapters/ft991a.rs` — boundary and branch gaps

**Where**: add to existing `#[cfg(test)]` module in `ft991a.rs`

**New tests:**

- `is_amateur_frequency_at_exact_band_edges` — test lower/upper edge of every band (1_800_000, 2_000_000, exact 60m edges, 2m, 70cm), plus one value just outside each
- `is_amateur_frequency_rejects_values_just_outside_bands` — 1_799_999, 2_000_001, 450_000_001
- `band_select_code_all_bands` — verify every band arm maps to the correct BS code (0–13); test the `_ =>` fallback with a non-amateur frequency
- `drop_impl_does_not_panic_when_ptt_fails` — construct a `FailingSerial` mock (write always returns `Err`), verify `Drop` runs without panic via `std::panic::catch_unwind`

**Estimated gain**: +6–8 lines, closes several uncovered branches in ft991a.rs

---

### Priority 2 — `domain/types.rs` — close remaining 29% gap

**Where**: add to existing `#[cfg(test)]` module in `types.rs`

**New tests:**

- `frequency_khz_constructor` — `Frequency::khz(14.070) == Frequency::hz(14_070.0)`
- `frequency_mhz_constructor` — `Frequency::mhz(14.070) == Frequency::hz(14_070_000.0)`
- `frequency_as_hz_roundtrip` — `Frequency::hz(7_035_000.0).as_hz() == 7_035_000.0`
- `modem_status_default` — `rx_running == false`, `carrier_freq_hz == 1000.0`
- `modem_config_default_tx_power` — `tx_power_watts == 25`
- `audio_device_info_serializes` — construct with all fields, verify `serde_json::to_string` produces camelCase keys (`isInput`, `isDefault`, `outputUnverified`)
- `radio_status_partial_eq` — identical structs are `==`, structs with differing `frequency_hz` are not
- `radio_info_serializes` — verify `frequencyHz` (not `frequency_hz`) in JSON output

**Estimated gain**: ~15 lines, pushing `domain/types.rs` from 71% to ~90%

---

### Priority 3 — `modem/decoder.rs` — close remaining 15%

**Where**: add to existing `#[cfg(test)]` module in `decoder.rs`

**New tests:**

- `signal_strength_with_high_gain` — feed silence until AGC max gain → `signal_strength()` close to 0.0
- `signal_strength_with_low_gain` — feed loud signal until AGC min gain → `signal_strength()` close to 1.0
- `reset_restores_decoder_to_initial_behavior` — run decoder to mid-stream state, call `reset()`, verify it can cleanly decode a fresh transmission from the beginning
- `phase_ambiguity_fallback_does_not_crash` — encode "X", partially decode to acquire lock, feed 200+ silence samples to trigger the phase ambiguity threshold, then feed a fresh encoding of "E" and verify the decoder is still functional

**Estimated gain**: +10–15 lines, from 85% to ~95%

---

### Priority 4 — `commands/config.rs` — close remaining 63%

**Current**: 37%. `sanitize_name` and `validate_tx_power` are already tested. The I/O path (`write_config_to_disk`, `load_configuration`, `list_configurations`, `delete_configuration`) requires an `AppHandle`.

**Approach**: Refactor the I/O logic into private fns that accept a `&Path` instead of `&AppHandle`, then test those directly with a `tempfile::tempdir()`. The Tauri commands become thin wrappers that resolve the path and delegate.

**New tests (after refactor):**

- `save_and_load_roundtrip` — save a `Configuration`, load it back, assert equal
- `list_returns_saved_names` — save two configs, `list_configurations` returns both
- `delete_removes_config` — save then delete, verify `load` returns `Err`
- `delete_default_returns_error` — `delete_configuration("Default")` always fails
- `load_nonexistent_returns_error`
- `validate_tx_power_at_boundaries` — 0W, 100W, 101W (if not already covered)

**Note**: Add `tempfile` to `[dev-dependencies]` in `Cargo.toml` for the temp dir helper.

**Estimated gain**: ~20–25 lines, from 37% to ~65%

---

### Priority 5 — `dsp/filter.rs` — missing branch paths

**Where**: add to existing `#[cfg(test)]` module

**New tests:**

- `bandpass_center_tap_is_nonzero` — construct a 127-tap bandpass, assert `coefficients[63] != 0.0` (exercises the `n == 0.0` branch inside coefficient generation)
- `fir_filter_circular_buffer_wraparound` — feed 10 samples through a 5-tap filter, verify output is finite and position cycles correctly

**Estimated gain**: +4–6 lines, from ~97% to ~99%

---

### Priority 6 — `cat/session.rs` — error and timeout paths

**Where**: add to existing `#[cfg(test)]` module using the existing `MockSerial` pattern

**New tests:**

- `zero_byte_reads_retry_until_semicolon` — `MockSerial` returns `Ok(0)` for first 5 reads, then `"FA00014070000;"` — `execute(GetFrequencyA)` succeeds
- `all_reads_timeout_returns_no_response_error` — `MockSerial` always returns `Ok(0)` — `execute(GetFrequencyA)` returns `Err` containing "no response"
- `transient_read_error_recovers` — `MockSerial` returns `Err` for 3 reads then returns a valid response
- `execute_write_only_sends_correct_wire_bytes` — verify `BandSelect(5)` writes `"BS05;"` and returns `Ok(())`
- `execute_write_only_propagates_write_error` — `FailingWriteMockSerial` causes `execute_write_only` to return `Err`

**Estimated gain**: +8–12 lines, covering the retry/timeout/write-only paths

---

### Priority 7 — Frontend E2E: branch coverage

**Where**: extend existing spec files or add new ones

**New tests:**

**`tests/e2e/serial.spec.ts`:**
- `connect at non-amateur frequency shows blank band selector` — mock `connect_serial` returning `frequencyHz: 10_000_000`, verify `#band-select` value is blank and freq input shows `10.000`
- `commitFreq ignores NaN input` — set freq input to `'abc'`, press Enter, verify `set_frequency` is never invoked
- `freq input with no active band sends unclamped value` — connect at non-amateur freq (band=null), enable freq input, type a value, press Enter, verify `set_frequency` is called with that raw value

**`tests/e2e/error-handling.spec.ts`:**
- `serial-disconnected with empty port shows generic label` — `fireEvent(page, 'serial-disconnected', { port: '' })`, verify toast says "CAT disconnected"
- `showToast warning type applies toast-warning class`
- `showToast info type applies toast-info class`

**`tests/e2e/settings.spec.ts`:**
- `corrupt waterfall palette in saved config falls back to classic` — mock `load_configuration` returning `waterfall_palette: 'invalid'`, open settings, verify palette select shows `classic`
- `corrupt zoom level in saved config falls back to 1x` — mock returning `waterfall_zoom: 3`, verify 1x button is active

**New file `tests/e2e/waterfall-controls.spec.ts`:**
- `zoom 2x centers visible range on carrier`
- `zoom 4x halves the visible range again`
- `switching palette updates the color map`
- `noise floor slider changes the displayed range`

**Estimated gain**: frontend branches from 59.4% to ~75%+

---

## Tests Section

### Rust
- **Run**: `cd src-tauri && cargo test`
- **Coverage**: `cargo llvm-cov --lib`
- **Pattern**: `#[cfg(test)] mod tests { ... }` in the same file as the code under test
- **New dev-dependency**: `tempfile` (for `commands/config.rs` I/O tests)
- **No new mocking crates**: use the existing `MockSerial` struct pattern

### Frontend
- **Run**: `npx playwright test`
- **Coverage**: `VITE_COVERAGE=true npx playwright test && npx nyc report`
- **Pattern**: import `{ test, expect }` from `./fixtures` (already done); use `mockInvoke` + `fireEvent` from `./helpers`
- **New spec files**: `tests/e2e/waterfall-controls.spec.ts`

---

## Realistic Targets After Implementation

| Module | Current | Target | Key additions |
|---|---|---|---|
| `adapters/ft991a.rs` | ~70% | ~88% | Band edge tests, Drop, band_select_code |
| `domain/types.rs` | 71% | ~90% | khz/mhz constructors, serialization |
| `modem/decoder.rs` | 85% | ~95% | signal_strength edges, phase ambiguity |
| `commands/config.rs` | 37% | ~65% | I/O via `&Path` refactor + temp dir |
| `dsp/filter.rs` | ~97% | ~99% | Center tap, circular buffer |
| `cat/session.rs` | ~70% | ~88% | Zero-byte reads, errors, write_only |
| `state.rs` | 83% | ~92% | Default impl assertions |
| **Rust overall** | **64%** | **~80%** | |
| Frontend branches | 59.4% | ~75% | 10+ new E2E branch tests |
| **Frontend lines** | **76.7%** | **~82%** | |

---

## Implementation Sequence

1. `domain/types.rs` — pure value tests, zero risk (Priority 2)
2. `adapters/ft991a.rs` pure functions — band edge tests, band_select_code (Priority 1a)
3. `cat/session.rs` — zero-byte/error/write_only paths (Priority 6)
4. `modem/decoder.rs` — signal_strength, phase ambiguity (Priority 3)
5. `dsp/filter.rs` — center tap, circular buffer (Priority 5)
6. Frontend: toast variants, empty-port serial event, zoom branches (Priority 7)
7. Frontend: serial-panel NaN / band=null / non-amateur connect paths (Priority 7)
8. `commands/config.rs` — refactor + I/O tests (Priority 4)
9. `adapters/ft991a.rs` Drop impl (Priority 1b)

---

## Constraints

- No new mocking crates (`mockall`, `rstest`, etc.) — use the `MockSerial` struct pattern already in `cat/session.rs`
- All frontend tests use `mockInvoke` + `fireEvent` from `tests/e2e/helpers.ts` — no new test infrastructure
- `commands/audio.rs`, `commands/serial.rs`, `commands/tx.rs`, `adapters/cpal_audio.rs`, `lib.rs`, `menu.rs` — do not attempt to cover these
- The 200ms `PORT_SETTLE_MS` sleep in `CatSession::new` is retained — do not add test-only bypasses
