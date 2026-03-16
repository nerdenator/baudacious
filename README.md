# Baudacious

Cross-platform desktop application for PSK-31 ham radio keyboard-to-keyboard communication.

## Features (implemented)

- Spectral waterfall display with click-to-tune (live FFT from audio input)
- Audio device enumeration and selection (input + output)
- CAT serial control for Yaesu FT-991A (connect, frequency, mode, PTT)
- TX text input with character counter and transmit/abort controls (UI ready, encoding in Phase 4)
- RX decoded text display with clear button
- Configuration profiles (save/load/delete)
- Native menu bar (File, View, Help)
- Light/dark theme with localStorage persistence

## Requirements

### Build Dependencies

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) 1.70+
- Platform-specific dependencies:
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)
  - **Linux**: `build-essential`, `libwebkit2gtk-4.1-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libasound2-dev`
  - **Windows**: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with C++ workload

### Runtime

- USB audio interface (radio or SignaLink)
- Serial port for CAT control (optional)

## Getting Started

```bash
# Install dependencies
npm install

# Run in development mode (hot reload)
npm run tauri dev

# Build for production
npm run tauri build
```

## Testing

```bash
# Rust unit tests (26 passing, 1 pre-existing failure in varicode roundtrip)
cd src-tauri && cargo test

# Playwright E2E tests (27 passing)
npm test

# Update visual regression snapshots after UI changes
npx playwright test --update-snapshots

# Check Rust compilation
cd src-tauri && cargo check
```

## Architecture

This project uses **hexagonal architecture** (ports & adapters) in the Rust backend to separate core domain logic from external I/O:

```
src-tauri/src/
├── domain/      # Pure types (AudioDeviceInfo, Frequency, ModemConfig, errors)
├── ports/       # Trait definitions (AudioInput, AudioOutput, RadioControl)
├── dsp/         # Signal processing (FFT, NCO, filters, Costas loop)
├── modem/       # PSK-31 protocol (varicode, encoder, decoder)
├── adapters/    # Implementations (cpal audio, serialport, FT-991A CAT)
├── commands/    # Tauri command handlers (audio, serial, radio, config)
├── menu.rs      # Native menu bar setup
└── state.rs     # AppState with Arc<Mutex<>>
```

### Frontend

Vanilla TypeScript + Vite, organized into modules:

```
src/
├── components/  # UI components (waterfall, serial-panel, audio-panel, etc.)
├── services/    # Backend API wrappers, event bridges
├── types/       # Shared TypeScript types matching Rust structs
├── utils/       # Color map, helpers
└── main.ts      # App entry, wiring
```

### Key Design Decisions

- **48000 Hz sample rate** — native USB audio rate for FT-991A
- **31.25 baud** — PSK-31 symbol rate (1536 samples/symbol)
- **Lock-free audio** — ring buffers between cpal callback and DSP thread
- **Pure DSP functions** — all signal processing is testable without hardware
- **Tauri events for streaming** — FFT data sent via `AppHandle::emit`, easy to mock in tests
- **Audio thread isolation** — `cpal::Stream` is `!Send`, so audio lives on a dedicated thread; AppState only holds an `AtomicBool` shutdown flag and `JoinHandle`

## Project Structure

```
psk31_client_workspace/
├── src/                    # Frontend (TypeScript)
│   ├── components/         # UI components
│   ├── services/           # Backend API, event bridges
│   ├── types/              # Type definitions
│   ├── utils/              # Helpers
│   └── main.ts             # App entry
├── src-tauri/              # Backend (Rust)
│   ├── src/
│   │   ├── domain/         # Core types
│   │   ├── ports/          # Trait interfaces
│   │   ├── dsp/            # Signal processing
│   │   ├── modem/          # PSK-31 protocol
│   │   ├── adapters/       # Hardware implementations
│   │   └── commands/       # Tauri IPC handlers
│   └── Cargo.toml
├── tests/e2e/              # Playwright E2E tests
├── PLAN.md                 # Master 6-phase implementation plan
├── PLAN_PHASE4_TX.md       # Phase 4 TX path plan
└── CLAUDE.md               # Development guidelines
```

## Roadmap

- [x] Phase 1: Project scaffolding, hexagonal module structure
- [x] Phase 1.5: Frontend layout, modular components, config persistence, E2E tests
- [x] Phase 2: Serial / CAT communication with FT-991A
- [x] Phase 3: Audio subsystem + live waterfall display
- [ ] Phase 4: PSK-31 TX path (encoder, modulator, audio output)
- [ ] Phase 5: PSK-31 RX path (demodulator, decoder, Costas loop)
- [ ] Phase 6: Integration + polish

See [PLAN.md](PLAN.md) for detailed implementation phases.

## License

MIT

## Acknowledgments

- Fonts: [IBM Plex Mono](https://github.com/IBM/plex) and [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) (SIL OFL 1.1)
- Inspired by [JS8Call](http://js8call.com/) and [WSJT-X](https://wsjt.sourceforge.io/)
