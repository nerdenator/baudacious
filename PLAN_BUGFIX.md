# Plan: Bug Fix Session

## Context

Post-code-review bug fix pass. Issues are prioritized by severity and grouped by subsystem. Each fix is small and self-contained. The goal is to land all fixes in one session with a clean commit per logical group.

---

## Issues to Fix

### Group A — TX / PTT Safety (Critical)

**A1. PTT lifecycle moved into TX thread** (`commands/tx.rs`)
- PTT is currently activated in `start_tx` before the thread spawns
- If `audio_output.start()` fails, PTT release depends on the frontend receiving an `error` event
- Fix: remove PTT ON from `start_tx`; move PTT ON to start of `run_tx_thread`, inside the thread body, before audio playback begins
- PTT OFF already happens at end of thread — no change needed there
- Removes the fragile frontend-as-safety-net dependency

**A2. Thread handle stored before PTT is activated** (`commands/tx.rs`)
- After A1, the thread is spawned first, then PTT fires inside the thread
- Ensure `state.tx_thread.lock().unwrap().replace(handle)` is placed immediately after `thread::spawn(...)` — before any `await`/`sleep`
- This closes the window where `stop_tx()` finds `tx_thread = None`
- A1 and A2 are done together as part of the same refactor

---

### Group B — Frontend Audio Listener Safety (Important)

**B1. Separate audio-status listener from FFT listener** (`audio-bridge.ts`)
- `stopFftBridge()` currently clears both `fftUnlisten` and `statusUnlisten`
- `startFftBridge()` calls `stopFftBridge()` before re-registering, silently killing the audio-status listener
- Fix: split into two independent functions — `stopFftBridge()` only clears `fftUnlisten`; add a new `stopAudioStatusBridge()` for `statusUnlisten`
- `listenAudioStatus` in `main.ts` is already separate — just ensure its unlisten is no longer tied to the FFT path

**B2. Decouple audio error handler from waterfall canvas** (`main.ts`)
- `listenAudioStatus()` is currently inside `if (waterfall) { ... }`
- Fix: move `listenAudioStatus(...)` outside the waterfall guard — it is independent of the waterfall display

---

### Group C — Rust Backend Polish (Important)

**C1. Use `with_radio()` in `start_tx`** (`commands/tx.rs`)
- `ensure_data_mode` and `set_tx_power` each lock `state.radio` directly and independently
- Fix: consolidate into a single `with_radio()` call that runs both operations, giving serial-error auto-disconnect for free
- If no radio is connected, skip both operations gracefully (already the current behavior)

**C2. Sanitize names in `list_configurations`** (`commands/config.rs`)
- `list_configurations` returns raw file stems without `sanitize_name`
- Fix: filter the results through `sanitize_name` and skip any entry where the stem differs from its sanitized form (i.e., silently ignore malformed filenames)

---

### Group D — Data & Display Quality

**D1. Preserve `tx_power_watts` in settings save** (`settings-dialog.ts`)
- The `onSave` handler constructs a config object that never includes `tx_power_watts`
- Fix: include `base?.tx_power_watts ?? 25` when constructing the saved config object

**D2. Replace `textContent +=` with `appendChild`** (`rx-display.ts`)
- `rxContentEl.textContent += text` re-serializes the entire element on every append
- Fix: use `rxContentEl.appendChild(document.createTextNode(text))` for O(1) appends

**D3. Fix `Object.values()` spread in waterfall controls** (`waterfall-controls.ts`)
- `updateScale(...Object.values(waterfall.getVisibleRange()) as [number, number])` relies on insertion order
- Fix: destructure by name — `const { startHz, endHz } = waterfall.getVisibleRange(); updateScale(startHz, endHz)`

---

### Group E — Cleanup

**E1. Remove stale `modem/pipeline.rs`** (`src-tauri/src/modem/pipeline.rs`)
- File contains only an outdated TODO comment referencing Phases 4 & 5 (both complete)
- Pipeline logic lives in `commands/audio.rs`
- Fix: delete the file and remove the `pub mod pipeline;` declaration from `modem/mod.rs`

---

## Commit Strategy

| Commit | Groups | Description |
|--------|--------|-------------|
| 1 | A1+A2 | Fix PTT lifecycle: activate inside TX thread, store handle before PTT |
| 2 | B1+B2 | Decouple audio-status listener from FFT bridge and waterfall guard |
| 3 | C1+C2 | Rust: consolidate start_tx radio locks via with_radio(); sanitize list_configurations |
| 4 | D1+D2+D3+E1 | Quality: preserve tx_power_watts, O(1) RX append, getVisibleRange destructure, remove pipeline stub |

---

## Out of Scope

- P7 (raised cosine phase flip timing) — already noted as deferred in CLAUDE.md
- P9 (CSP null) — already noted as deferred in CLAUDE.md
- `setSelectedAudioDevices` null-op (issue #3 from review) — safe in current call order; not worth the defensive complexity

## Status

- [x] A: TX / PTT safety
- [x] B: Frontend audio listener safety
- [x] C: Rust backend polish
- [x] D: Data & display quality
- [ ] E: Cleanup — DEFERRED
