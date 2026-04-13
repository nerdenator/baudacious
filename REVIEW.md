# Code Review Guide — Baudacious

This document is written for an AI code reviewer (GitHub Copilot or similar). It provides the context needed to do a meaningful review of this codebase without hardware access.

## What This Project Is

**Baudacious** is a cross-platform desktop app for PSK-31 ham radio keyboard-to-keyboard communication. PSK-31 is a narrow-band digital mode that encodes text using Binary Phase Shift Keying (BPSK) at 31.25 baud.

- **Runtime**: Tauri 2.x (Rust backend + web frontend in a single binary)
- **Target hardware**: Yaesu FT-991A via USB audio (48kHz) + CAT serial (38400 baud)
- **Frontend**: Vanilla TypeScript + Vite (no framework)
- **Backend**: Rust with hexagonal (ports & adapters) architecture

The app lets a ham radio operator type a message, encode it as PSK-31, transmit it over the air via the radio, and receive/decode incoming PSK-31 signals — all with a live waterfall spectrogram display.

## Implementation Status

All 6 original phases are complete:

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Project scaffolding, hexagonal module structure | ✅ |
| 1.5 | Frontend layout, modular components, config persistence, E2E tests | ✅ |
| 2 | Serial / CAT communication with FT-991A | ✅ |
| 3 | Audio subsystem + live waterfall FFT display | ✅ |
| 4 | PSK-31 TX path (encoder → modulator → audio output) | ✅ |
| 5 | PSK-31 RX path (demodulator → decoder → text display) | ✅ |
| 6 | Integration + polish (error handling, settings, E2E, packaging) | ✅ |

**Currently in progress**: Phase 9A — CAT command translation module (`src-tauri/src/cat/`)

## Architecture Overview

### Rust Backend (Hexagonal)

```
src-tauri/src/
├── domain/          # Pure types — no I/O, no side effects
│   ├── types.rs     # AudioDeviceInfo, Frequency, ModemConfig
│   ├── config.rs    # Configuration profile structs
│   └── error.rs     # Psk31Error enum (thiserror)
│
├── ports/           # Trait definitions (the "hexagon" boundary)
│   ├── audio.rs     # AudioInput, AudioOutput traits
│   ├── serial.rs    # SerialFactory, SerialConnection traits
│   └── radio.rs     # RadioControl trait
│
├── dsp/             # Signal processing — pure functions, all unit-tested
│   ├── fft.rs       # FFT via rustfft, windowed with Hann
│   ├── nco.rs       # Numerically controlled oscillator
│   ├── agc.rs       # Automatic gain control
│   ├── costas_loop.rs    # Carrier phase/frequency tracking
│   ├── clock_recovery.rs # Mueller-Muller symbol timing
│   ├── filter.rs    # IIR filters
│   └── raised_cosine.rs  # Pulse shaping for TX
│
├── modem/           # PSK-31 protocol
│   ├── varicode.rs  # Varicode encode/decode tables
│   ├── encoder.rs   # Text → Varicode bits → BPSK samples
│   ├── decoder.rs   # BPSK samples → Varicode bits → text
│   └── pipeline.rs  # Stub (TODO) — RX pipeline logic lives in decoder.rs
│
├── cat/             # CAT command translation layer (Phase 9A, in progress)
│   ├── encode.rs    # Rust types → FT-991A CAT ASCII strings
│   ├── decode.rs    # CAT ASCII responses → Rust types
│   └── session.rs   # Stateful CAT conversation
│
├── adapters/        # Concrete I/O implementations
│   ├── cpal_audio.rs     # CPAL audio adapter (AudioInput + AudioOutput)
│   ├── serial_port.rs    # serialport crate adapter
│   ├── ft991a.rs         # Yaesu FT-991A CAT control (RadioControl impl)
│   └── mock_radio.rs     # In-memory mock for testing
│
├── commands/        # Tauri IPC command handlers (driving adapters)
│   ├── audio.rs     # list_audio_devices, start/stop_audio_stream, start/stop_rx, set_carrier_frequency
│   ├── serial.rs    # list_serial_ports, connect/disconnect_serial
│   ├── radio.rs     # ptt_on/off, get/set_frequency, get/set_mode
│   ├── tx.rs        # start_tx, stop_tx
│   ├── config.rs    # save/load/list/delete_configuration
│   └── status.rs    # get_connection_status
│
├── state.rs         # AppState — all shared mutable state behind Arc<Mutex<>>
├── menu.rs          # Native menu bar (Tauri MenuBuilder)
└── lib.rs           # App entry, plugin init, command registration
```

### TypeScript Frontend

```
src/
├── components/          # UI components — each owns its DOM section
│   ├── waterfall.ts     # Canvas waterfall spectrogram
│   ├── waterfall-controls.ts  # Palette/zoom/noise floor knobs
│   ├── rx-display.ts    # Decoded text pane, auto-scroll, clear
│   ├── tx-input.ts      # TX text input, transmit/abort buttons
│   ├── serial-panel.ts  # Band selector, frequency input, mode display
│   ├── audio-panel.ts   # Device selectors + refresh
│   ├── settings-dialog.ts  # Tabbed modal (General/Audio/Radio)
│   ├── status-bar.ts    # Connection indicator dots
│   └── toast.ts         # Slide-in notification toasts
│
├── services/            # State + IPC — components import these, not each other
│   ├── backend-api.ts   # Typed wrappers around Tauri invoke()
│   ├── app-state.ts     # Pub/sub config state (persisted to disk)
│   ├── audio-bridge.ts  # FFT event subscription, audio state
│   ├── rx-bridge.ts     # rx-text event subscription
│   ├── tx-bridge.ts     # tx-status event subscription + TX control
│   ├── serial-bridge.ts # serial-disconnected events, radio state
│   └── event-handlers.ts  # Central Tauri event router
│
├── types/index.ts       # TypeScript interfaces mirroring Rust structs
└── utils/
    ├── color-map.ts     # Waterfall palette LUTs (classic/heat/viridis/grayscale)
    └── formatter.ts     # Text helpers
```

## Key Design Decisions

### Audio Thread Architecture

`cpal::Stream` is `!Send`, so it cannot be stored in `AppState`. Instead:
- A dedicated OS thread owns the `CpalAudioInput` + DSP pipeline + ring buffer
- `AppState` holds only `Arc<AtomicBool>` (shutdown flag) and a `JoinHandle`
- FFT results stream to the frontend via `AppHandle::emit("fft-data", ...)`
- Decoded RX text streams via `AppHandle::emit("rx-text", ...)`

### CAT / Radio Commands

The `with_radio()` helper in `commands/radio.rs` abstracts the common lock → check → call pattern for all radio commands. When a `Psk31Error::Serial` is returned, it nulls `AppState.radio` and emits a `serial-disconnected` event so the frontend can reset UI state.

### TX Flow

1. `start_tx` encodes the full message text to BPSK samples upfront (not streaming)
2. Preamble (32 idle bits) → message → postamble (32 idle bits)
3. PTT ON → 50ms delay → playback via `CpalAudioOutput` → PTT OFF
4. If no radio is connected, PTT commands are skipped (audio-only mode)

### RX Pipeline

The RX DSP chain runs inside the existing audio thread:

```
Raw samples → AGC → Costas Loop (carrier tracking) → Mueller-Muller clock recovery → differential bit decode → Varicode → text
```

Key tuning notes:
- Costas Loop gains: `Kp=0.01`, `Ki=0.000005` (empirically tuned — textbook values caused instability)
- Symbol squelch at `0.001` to suppress noise during lock acquisition
- First character of a transmission is typically lost during Costas Loop lock (normal PSK-31 behavior)
- Phase ambiguity fallback: invert bit sense after 100 bits without a valid Varicode character

### Frontend State Flow

Components never call Tauri directly — they go through services:

```
Component → backend-api.ts (typed invoke) → Tauri IPC → Rust command
                                          ← Tauri event ← Rust emit
Component ← *-bridge.ts (typed subscriber) ←────────────┘
```

Config state (`app-state.ts`) is pub/sub: components subscribe to changes instead of polling.

## Test Coverage

### Rust Unit Tests

Each DSP/modem module has `#[cfg(test)]` blocks. Run with:

```bash
cd src-tauri && cargo test
```

Key test locations:
- `dsp/fft.rs` — FFT correctness, peak detection
- `dsp/nco.rs` — Phase accumulation, phase wrapping
- `dsp/costas_loop.rs` — Phase tracking convergence
- `dsp/clock_recovery.rs` — Mueller-Muller timing
- `modem/varicode.rs` — Encode/decode round-trips
- `modem/encoder.rs` — BPSK sample generation
- `modem/decoder.rs` — Bit decision logic

### Integration Tests

```bash
cd src-tauri && cargo test --test rx_loopback
cd src-tauri && cargo test --test cat_integration
```

- `tests/rx_loopback.rs` — Full TX→RX loopback (encoder output → decoder)
- `tests/cat_integration.rs` — CAT command encode/decode round-trips

### Playwright E2E Tests (50+)

```bash
npm test
```

Key files in `tests/e2e/`:
- `helpers.ts` — Shared `mockInvoke` + `fireEvent` infrastructure
  - Tauri 2.x event mock: uses `transformCallback` + `plugin:event|listen` (not Tauri 1.x `window.dispatchEvent`)
  - `fireEvent(page, eventName, payload)` routes through registered `listen()` handlers
- `app.spec.ts` — Visual regression (dark/light theme snapshots)
- `serial.spec.ts` — Band selector, frequency input, CAT connect/disconnect
- `audio.spec.ts` — Device list, start/stop stream
- `rx.spec.ts` — RX text, click-to-tune, clear
- `settings.spec.ts` — Tabbed dialog, profile CRUD
- `error-handling.spec.ts` — Serial/audio error flows, toast notifications

## Areas to Focus the Review On

The following areas have complexity worth examining closely:

1. **`src-tauri/src/dsp/costas_loop.rs`** — PLL implementation; gains are empirically tuned and the rationale is only in comments/memory
2. **`src-tauri/src/modem/decoder.rs`** — Orchestrates the full RX chain (AGC → Costas Loop → Clock Recovery → Varicode); check that state resets correctly on `set_carrier_frequency`
3. **`src-tauri/src/state.rs`** — All shared state; check for lock ordering, potential deadlocks, and that `Arc<AtomicBool>` flags are used correctly
4. **`src-tauri/src/commands/radio.rs`** — `with_radio()` helper and error→event propagation path
5. **`src/services/app-state.ts`** — Pub/sub config; check subscriber cleanup and initialization ordering
6. **`src/components/settings-dialog.ts`** — `dialogConfig` snapshot pattern; profile switch vs. save race conditions were previously fixed
7. **`src-tauri/src/cat/`** — New module (Phase 9A, in progress); encode/decode correctness against FT-991A CAT reference

## Known Limitations / Accepted Tradeoffs

- First character of RX transmission is lost during Costas Loop lock acquisition — this is normal PSK-31 behavior, not a bug
- `settings-dialog.ts` registers an `Escape` key listener on `document` that leaks on hot-reload (low priority, dev-only)
- `serialport` dependency is MPL-2.0; if direct modification of that crate is needed, consider switching to `tokio-serial` (MIT)
- RX decoder has a phase ambiguity fallback (invert bit sense after 100 consecutive invalid chars) — may cause a momentary glitch on mode switch

## Running Locally (No Radio Required)

The app functions without hardware connected:

```bash
npm install
npm run tauri dev   # Hot reload dev mode
```

- **Without serial**: CAT controls remain disabled; TX still works with audio only
- **Without audio**: Waterfall shows no data; simulated FFT mode can be toggled in code
- **All E2E tests**: Run entirely against mocked Tauri IPC — no hardware or radio needed

```bash
npm test            # All Playwright tests
cd src-tauri && cargo test   # All Rust tests
```
