# Plan: Pipeline Refactor — Move Domain Logic Out of Commands Layer

## Status

- [x] R1: Move carrier retune detection into `Psk31Decoder`
- [x] R2: Verify phase ambiguity fallback ownership (already in decoder — confirm and close)
- [x] R3: Delete `modem/pipeline.rs` stub
- [x] R4: Audit `commands/tx.rs` for domain logic leakage

> **Note**: R3 supersedes PLAN_BUGFIX.md item E1. E1 is marked deferred in that plan;
> this plan owns it. Do not do E1 separately.

---

## Background

Hexagonal architecture requires that domain logic live in the domain/modem layer and that
the adapter (commands) layer do nothing more than wire ports together. Currently, two files
violate this boundary:

- `src-tauri/src/commands/audio.rs` — owns carrier retune detection logic
- `src-tauri/src/modem/pipeline.rs` — dead stub polluting the module tree

`commands/tx.rs` is examined in R4 for completeness; it is expected to be clean.

---

## R1 — Move Carrier Retune Detection into `Psk31Decoder`

### Problem

`commands/audio.rs` lines 226–231 manually compare the current carrier frequency against a
locally-held `current_carrier` copy, and if a change is detected it calls
`decoder.set_carrier_freq(target_carrier)`:

```rust
// commands/audio.rs: lines 197–199 (setup) + 226–231 (loop)
let initial_carrier = *rx_carrier_freq.lock().unwrap();
let mut decoder = Psk31Decoder::new(initial_carrier, sample_rate);
let mut current_carrier = initial_carrier;
...
let target_carrier = *rx_carrier_freq.lock().unwrap();
if (target_carrier - current_carrier).abs() > 0.1 {
    decoder.set_carrier_freq(target_carrier);
    current_carrier = target_carrier;
}
```

This is domain logic (what constitutes a "significant" frequency change, and which
subsystems need resetting) leaking into the adapter layer. The 0.1 Hz threshold and the
shadow `current_carrier` variable both belong inside `Psk31Decoder`.

### Target State

`Psk31Decoder` exposes a method:

```rust
pub fn update_carrier_if_changed(&mut self, freq: f64)
```

This method internally holds `self.carrier_freq` (already a field, see `decoder.rs` line 44)
and applies the 0.1 Hz threshold check. If the frequency has changed beyond the threshold, it
calls `self.set_carrier_freq(freq)`. The commands layer drops its `current_carrier` shadow
variable entirely and calls:

```rust
decoder.update_carrier_if_changed(*rx_carrier_freq.lock().unwrap());
```

The full reset logic (Costas loop reset, clock recovery rebuild, varicode reset, clear
last_symbol / bits_without_char / invert_bits) stays exactly where it is — inside
`set_carrier_freq` at `decoder.rs` lines 115–124. Only the change-detection gate moves.

### Implementation Steps

1. Add `update_carrier_if_changed(&mut self, freq: f64)` to `Psk31Decoder` in
   `src-tauri/src/modem/decoder.rs`. Implementation body:
   ```rust
   pub fn update_carrier_if_changed(&mut self, freq: f64) {
       if (freq - self.carrier_freq).abs() > 0.1 {
           self.set_carrier_freq(freq);
       }
   }
   ```
2. In `src-tauri/src/commands/audio.rs`:
   - Remove the `let mut current_carrier = initial_carrier;` binding (line 199).
   - Replace the block at lines 226–231 with a single call:
     `decoder.update_carrier_if_changed(*rx_carrier_freq.lock().unwrap());`
3. Add a unit test in `decoder.rs` that verifies `update_carrier_if_changed` does not reset
   state when the delta is below 0.1 Hz, and does reset when it exceeds 0.1 Hz.

### Verification

- `cargo test` passes with no regressions.
- `commands/audio.rs` no longer holds any carrier-frequency comparison logic.
- The test in step 3 covers both the no-op and the retune paths.

### Risk

Low. `set_carrier_freq` is already tested (decoder.rs line 229–241). The only change is
adding a thin wrapper; no DSP logic is altered. The audio thread budget is unaffected — the
mutex lock and float comparison already existed.

---

## R2 — Phase Ambiguity Fallback Ownership (Verification)

### Problem to Verify

A prior review raised the concern that `commands/audio.rs` might be tracking
`bits_without_char` and `invert_bits` outside the decoder. If so, that state belongs in
`Psk31Decoder`.

### Findings (from reading source)

`Psk31Decoder` already owns this fully:

- `bits_without_char: usize` — field at `decoder.rs` line 38
- `invert_bits: bool` — field at `decoder.rs` line 41
- Phase ambiguity fallback logic — `decoder.rs` lines 101–106
- Constants `PHASE_AMBIGUITY_THRESHOLD` and `SYMBOL_SQUELCH` — `decoder.rs` lines 21–25

`commands/audio.rs` has zero references to phase ambiguity, bit inversion, or fallback logic.

### Action

No code changes required. This item is closed by inspection.

Document this finding as a comment in `decoder.rs` above `PHASE_AMBIGUITY_THRESHOLD` if it
is not already clear that this is intentionally encapsulated (it is — no comment needed; the
existing doc comment on `bits_without_char` and the inline comments in `process()` are
sufficient).

### Verification

Search `commands/audio.rs` for `invert`, `bits_without_char`, `PHASE_AMBIGUITY`. Expect zero
matches.

---

## R3 — Delete `modem/pipeline.rs` Stub

### Problem

`src-tauri/src/modem/pipeline.rs` contains only a single comment:

```rust
//! Modem Pipeline - TODO: Implement in Phases 4 & 5
```

Phases 4 and 5 are complete. The pipeline was implemented inline in `commands/audio.rs`
(not in this file). The stub is dead code that creates noise in the module tree and implies
architectural intent that was never realized.

`src-tauri/src/modem/mod.rs` line 8 declares `pub mod pipeline;`, making the stub part of
the public API. It exports nothing.

### Target State

- `src-tauri/src/modem/pipeline.rs` is deleted.
- `pub mod pipeline;` is removed from `src-tauri/src/modem/mod.rs`.
- No other files reference `modem::pipeline` (verified before deletion).

### Implementation Steps

1. Search for any usage: `grep -r "pipeline" src-tauri/src/`. Expect zero hits outside
   `modem/mod.rs` and `modem/pipeline.rs` themselves.
2. Delete `src-tauri/src/modem/pipeline.rs`.
3. Remove line 8 (`pub mod pipeline;`) from `src-tauri/src/modem/mod.rs`.
4. Run `cargo check` to confirm no dangling references.

### Verification

- `cargo check` passes.
- `cargo test` passes.
- `modem/mod.rs` no longer declares `pipeline`.

### Risk

None. The file is a single-line comment with no exports. This is a mechanical deletion.

### Supersedes

PLAN_BUGFIX.md item E1. That item was marked deferred; this plan owns it.

---

## R4 — Audit `commands/tx.rs` for Domain Logic Leakage

### Audit Scope

Review `commands/tx.rs` for any encoding or DSP logic that belongs in `Psk31Encoder` or the
domain layer rather than the adapter layer.

### Findings (from reading source)

**`data_mode_for_frequency(hz: f64)` — lines 34–44**

This function determines LSB vs USB based on frequency. It applies FCC Part 97 rules (60m
USB exception, below-10-MHz LSB convention). This is radio-protocol domain knowledge.

However, it is currently a free function in `commands/tx.rs` that is only called from
`ensure_data_mode` (line 59), which is itself only called from `start_tx`. There is no
modem struct to hang this on — it is radio CAT behavior, not encoder behavior. The correct
home for it would be `src-tauri/src/adapters/ft991a.rs` or a new
`src-tauri/src/domain/radio.rs` module, not `Psk31Encoder`.

Moving it is out of scope for this plan (it has no architectural impact on the
encoder/decoder boundary and requires touching CAT/radio domain types). Flag for a future
radio-domain cleanup pass.

**`Psk31Encoder` usage — lines 104–105**

```rust
let encoder = Psk31Encoder::new(sample_rate, carrier_freq);
let samples = encoder.encode(&text);
```

This is clean. The encoder is created with its parameters, called once, and the result is
handed off to the audio thread. No encoding logic leaks into the commands layer.

**TX thread (`run_tx_thread`) — lines 168–306**

The thread handles PTT, audio playback lifecycle, progress tracking, and event emission.
None of this is DSP logic. It is adapter-layer coordination (hardware control + IPC). This
is the correct home for it.

### Action

No code changes required for R4. Document `data_mode_for_frequency` as a candidate for
future radio-domain cleanup.

---

## Commit Strategy

| # | Covers | Files Changed | Description |
|---|--------|---------------|-------------|
| 1 | R3 | `modem/pipeline.rs` (delete), `modem/mod.rs` | Remove dead pipeline stub |
| 2 | R1 | `modem/decoder.rs`, `commands/audio.rs` | Add `update_carrier_if_changed`; remove shadow carrier var from audio thread |
| 3 | R2, R4 | None (no code changes) | (No commit needed — both items closed by inspection) |

Commit order: R3 first (pure deletion, zero risk), then R1 (smallest surface-area behavior
change). Each commit leaves the app in a working, test-passing state.

---

## Out of Scope

- **P7** (encoder phase flip timing) — already tracked as deferred in CLAUDE.md
- **P9** (CSP null) — already tracked as deferred in CLAUDE.md
- **`data_mode_for_frequency` relocation** — noted in R4 audit; deferred to a radio-domain
  cleanup pass
- Any changes to the audio thread architecture (ring buffer, FFT processing, signal-level
  emission) — these are adapter-layer concerns correctly placed in `commands/audio.rs`
