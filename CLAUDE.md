# PSK-31 Desktop Client

Cross-platform desktop application for PSK-31 ham radio keyboard-to-keyboard communication.

## Tech Stack

- **Framework**: Tauri 2.x (Rust backend + web frontend)
- **Frontend**: Vanilla TypeScript + Vite
- **Backend**: Rust with hexagonal architecture
- **Target Radio**: Yaesu FT-991A (USB audio + CAT serial)

## Architecture

Hexagonal (ports & adapters) pattern in the Rust backend:

```
src-tauri/src/
├── domain/      # Pure types (AudioDeviceInfo, Frequency, ModemConfig, errors)
├── ports/       # Trait definitions (AudioInput, AudioOutput, SerialConnection, RadioControl)
├── dsp/         # Signal processing - pure functions (FFT, NCO, filters, Costas loop)
├── modem/       # PSK-31 protocol (varicode, encoder, decoder, pipeline)
├── adapters/    # Implementations (cpal audio, serialport, FT-991A CAT)
├── commands/    # Tauri command handlers
└── state.rs     # AppState with Arc<Mutex<>>
```

## Key Files

- `PLAN.md` — Full implementation plan with 6 phases
- `PLAN_MENU.md` — Native menu bar implementation plan
- `src-tauri/src/lib.rs` — Tauri app builder, command registration
- `src-tauri/src/menu.rs` — Native menu bar setup and event handling
- `src-tauri/src/domain/config.rs` — Configuration profile types
- `src-tauri/src/modem/varicode.rs` — PSK-31 Varicode tables (complete)
- `src-tauri/src/dsp/fft.rs` — FFT processor with tests
- `src-tauri/src/dsp/nco.rs` — Numerically controlled oscillator with tests

## Design Philosophy

- **Inspired by**: JS8Call, WSJT-X — clean, functional, no clutter
- **Not like**: fldigi — we do ONE thing (PSK-31 keyboard QSOs) well
- Dark theme, waterfall prominent, monospace fonts for RX/TX

## Commands

```bash
npm run tauri dev      # Development with hot reload
cargo check            # Check Rust compilation (in src-tauri/)
cargo test             # Run Rust unit tests (in src-tauri/)
```

## Testing

- **Rust unit tests**: `#[cfg(test)]` modules in each file
- **Playwright**: Visual regression tests with mocked Tauri IPC (Phase 1.5)
- **Integration**: TX→RX loopback test (encoder output → decoder)

## Current Status

- [x] Phase 1: Project scaffolding, hexagonal module structure
- [x] Phase 1.5: Frontend layout & visual tests
- [x] Phase 2: Serial / CAT communication with FT-991A
- [x] Phase 3: Audio subsystem + waterfall display
- [x] Phase 4: PSK-31 TX path
- [x] Phase 5: PSK-31 RX path
- [x] Phase 6: Integration + polish (status bar ✓, error handling ✓, waterfall controls ✓, settings dialog ✓, E2E tests ✓, bundler packaging ✓)

## Instructions for Claude

- **When completing a phase**: Update the Current Status checklist above and commit the change
- **When starting a phase**: Mark the task in_progress using TaskUpdate

## Known Deferred Items

- **P7 — Encoder phase flip timing** (`src-tauri/src/modem/encoder.rs`): Phase is flipped at the start of each symbol (full amplitude), defeating raised cosine shaping. Correct fix requires splitting the envelope across the symbol boundary (look-ahead). Marked with TODO in source.
- **P9 — CSP disabled** (`src-tauri/tauri.conf.json`): `"csp": null` disables Content Security Policy. Low risk for a local-only app but worth hardening. Replace with `"default-src 'self'; style-src 'self' 'unsafe-inline'"` when ready.

## License Consideration

All dependencies MIT/Apache-2.0 except `serialport` (MPL-2.0). If we need to modify serialport, consider `tokio-serial` (MIT) instead.
