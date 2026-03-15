# Plan: Phase 9 — CAT Expansion

## Context

The current `RadioControl` trait covers a minimal slice: PTT, get/set frequency,
get/set mode. Real PSK-31 operation requires more. This plan also introduces a
dedicated **CAT translation module** that cleanly separates the two concerns
currently conflated inside `Ft991aRadio`:

1. **Translation** — mapping domain values to exact CAT strings, and parsing
   CAT responses back to domain values (pure functions, no I/O)
2. **Transport** — sending bytes over serial and reading responses (I/O only)

---

## CAT Protocol Basics (from FT-991A CAT manual)

Every command is: `[2-char mnemonic][parameters];`

Three command types:
- **Set** — sent from computer to radio to change a setting (`FA00014070000;`)
- **Read** — query sent from computer (`FA;`)
- **Answer** — radio's response to a Read (`FA00014070000;`)

Parameter widths are **exact and predetermined** per command. The radio
responds with `?` for any of:
- Too few digits
- Too many digits
- Extra characters between parameters
- Command not applicable in current radio state

Key commands for PSK-31 operation:

| Mnemonic | Read | Set / Answer format | Notes |
|----------|------|---------------------|-------|
| `FA` | `FA;` | `FA00014070000;` | VFO-A freq, 11-digit Hz, zero-padded |
| `FB` | `FB;` | `FB00014070000;` | VFO-B freq, same format |
| `MD0` | `MD0;` | `MD0C;` | Mode code (C = DATA-USB) |
| `TX` | — | `TX0;` / `TX1;` / `TX2;` | RX / PTT-mic / PTT-data |
| `PC` | `PC;` | `PC050;` | TX power 0–100W, 3-digit |
| `SM0` | `SM0;` | `SM00015;` | S-meter 0–30 scale, 5-digit answer |
| `IF` | `IF;` | `IF{37 chars};` | Comprehensive status (freq, mode, PTT, RIT, split) |
| `FT` | `FT;` | `FT0;` / `FT1;` | TX on VFO-A / VFO-B (split) |
| `RT` | `RT;` | `RT0;` / `RT1;` | RIT off / on |
| `RC` | — | `RC;` | Clear RIT offset |
| `RD` | — | `RD0100;` | RIT down N×10 Hz, 4-digit |
| `RU` | — | `RU0100;` | RIT up N×10 Hz, 4-digit |
| `IS0` | `IS0;` | `IS0+1000;` | IF shift, signed 4-digit Hz |
| `SH0` | `SH0;` | `SH00;` | IF width code, 2-digit (table per mode) |
| `NA0` | `NA0;` | `NA00;` / `NA01;` | Narrow mode off / on |
| `RM` | `RM1;`…`RM6;` | `RM10015;` | Meter: 1=ALC, 5=SWR, 6=PO |

`IF` response layout (37 chars before `;`):
```
IF P1(11) P2(5space) P3(5) P4(1) P5(1) P6(2space) P7(1) P8(2) P9(1) P10(1) P11(1) P12(1);
   ^freq            ^rit  ^ron  ^xon  ^mode        ...
```

---

## New Architecture: `cat/` Module

### Problem with current design

`Ft991aRadio` conflates translation and transport. Testing translation
(e.g. "does 7.035 MHz produce `FA00007035000;`?") requires a `MockSerial`.
Testing transport (e.g. "does a `?` response return `Err`?") requires knowing
the string format. They're hard to pull apart.

### Solution: split into three layers

```
src-tauri/src/cat/
├── mod.rs          # CatCommand enum, CatResponse enum, re-exports
├── encode.rs       # pure fn encode(cmd: &CatCommand) -> String
├── decode.rs       # pure fn decode(response: &str, cmd: &CatCommand) -> CatResponse
└── session.rs      # CatSession: owns SerialConnection, executes commands
```

**`CatCommand`** — typed enum for every supported operation:
```rust
pub enum CatCommand {
    // VFO
    GetFrequencyA,
    SetFrequencyA(u64),
    GetFrequencyB,
    SetFrequencyB(u64),
    // Mode
    GetMode,
    SetMode(String),
    // PTT
    PttOff,
    PttOn,
    // Power
    GetTxPower,
    SetTxPower(u32),
    // Meters
    GetSignalStrength,
    GetAlc,
    GetSwr,
    GetPowerOutput,
    // Status
    GetStatus,          // IF;
    // Split
    GetSplit,
    SetSplit(bool),
    // RIT
    GetRit,
    SetRit(bool),
    ClearRit,
    RitDown(u32),       // steps of 10 Hz
    RitUp(u32),
    // IF controls
    GetIfShift,
    SetIfShift(i32),    // Hz, signed
    GetIfWidth,
    SetIfWidth(u32),    // mode-specific code
    SetNarrow(bool),
}
```

**`CatResponse`** — typed enum for every possible answer:
```rust
pub enum CatResponse {
    FrequencyHz(u64),
    Mode(String),
    TxPower(u32),
    SignalStrength(f32),   // normalised 0.0–1.0 from SM0 0–30 scale
    Status(RadioStatus),
    Split(bool),
    Rit(bool),
    IfShiftHz(i32),
    IfWidthCode(u32),
    Narrow(bool),
    Ack,                   // command accepted, response was just ";"
}
```

**`encode(cmd: &CatCommand) -> String`** — pure function, zero I/O:
```rust
pub fn encode(cmd: &CatCommand) -> String {
    match cmd {
        CatCommand::GetFrequencyA       => "FA;".to_string(),
        CatCommand::SetFrequencyA(hz)   => format!("FA{hz:011};"),
        CatCommand::GetMode             => "MD0;".to_string(),
        CatCommand::SetMode(code)       => format!("MD0{code};"),
        CatCommand::PttOff              => "TX0;".to_string(),
        CatCommand::PttOn               => "TX1;".to_string(),
        CatCommand::GetTxPower          => "PC;".to_string(),
        CatCommand::SetTxPower(w)       => format!("PC{w:03};"),
        CatCommand::GetSignalStrength   => "SM0;".to_string(),
        CatCommand::GetStatus           => "IF;".to_string(),
        // ... etc
    }
}
```

**`decode(response: &str, cmd: &CatCommand) -> Psk31Result<CatResponse>`** — pure:
```rust
pub fn decode(response: &str, cmd: &CatCommand) -> Psk31Result<CatResponse> {
    if response.trim() == "?" {
        return Err(Psk31Error::Cat(format!(
            "Radio rejected command '{}'", encode(cmd)
        )));
    }
    match cmd {
        CatCommand::GetFrequencyA | CatCommand::SetFrequencyA(_) => {
            // parse "FA00014070000;" → FrequencyHz(14_070_000)
        }
        CatCommand::GetTxPower => {
            // parse "PC050;" → TxPower(50)
        }
        CatCommand::GetSignalStrength => {
            // parse "SM00015;" → SignalStrength(0.5)
        }
        // ...
        _ => Ok(CatResponse::Ack),
    }
}
```

**`CatSession`** — the only layer that touches I/O:
```rust
pub struct CatSession {
    serial: Box<dyn SerialConnection>,
    last_command_time: Option<Instant>,
}

impl CatSession {
    pub fn execute(&mut self, cmd: &CatCommand) -> Psk31Result<CatResponse> {
        self.ensure_command_delay();
        let wire_str = encode(cmd);
        log::debug!("CAT TX: {wire_str}");
        self.serial.write(wire_str.as_bytes())?;
        let raw = self.read_response(&wire_str)?;
        log::debug!("CAT RX: {raw}");
        decode(&raw, cmd)
    }
}
```

**`Ft991aRadio`** becomes a thin adapter:
```rust
pub struct Ft991aRadio {
    session: CatSession,
    is_transmitting: bool,
    is_split: bool,
}

impl RadioControl for Ft991aRadio {
    fn get_frequency(&mut self) -> Psk31Result<Frequency> {
        match self.session.execute(&CatCommand::GetFrequencyA)? {
            CatResponse::FrequencyHz(hz) => Ok(Frequency::hz(hz as f64)),
            _ => Err(Psk31Error::Cat("unexpected response".into())),
        }
    }
    fn set_tx_power(&mut self, watts: u32) -> Psk31Result<()> {
        self.session.execute(&CatCommand::SetTxPower(watts))?;
        Ok(())
    }
    // ...
}
```

### Why this is better

| Concern | Old | New |
|---------|-----|-----|
| Test "does 40m produce FA00007035000;?" | Needs MockSerial | Just call `encode()` |
| Test "does `?` return Err?" | Needs MockSerial with `?` response | Just call `decode("?", &cmd)` |
| Test transport timing | Coupled to string format | `CatSession` tested alone with MockSerial |
| Add a new command | Edit Ft991aRadio directly | Add enum variant + encode/decode case |

---

## Session A: Foundation (cat/ module + safety)

### A1: Create `cat/` module

- `cat/mod.rs`: `CatCommand`, `CatResponse` enums
- `cat/encode.rs`: `encode()` pure function covering all current commands
- `cat/decode.rs`: `decode()` pure function covering all current commands + `?` handling
- `cat/session.rs`: `CatSession` replacing the raw serial I/O in `Ft991aRadio`
- Refactor `Ft991aRadio` to delegate to `CatSession`
- All existing tests must still pass

**New unit tests** (`cat/encode.rs` and `cat/decode.rs` — no MockSerial needed):
```rust
#[test] fn encode_set_frequency_a() {
    assert_eq!(encode(&CatCommand::SetFrequencyA(14_070_000)), "FA00014070000;");
}
#[test] fn encode_set_tx_power_pads_to_3_digits() {
    assert_eq!(encode(&CatCommand::SetTxPower(25)), "PC025;");
}
#[test] fn decode_question_mark_is_err() {
    assert!(decode("?", &CatCommand::GetFrequencyA).is_err());
}
#[test] fn decode_frequency_answer() {
    let r = decode("FA00014070000;", &CatCommand::GetFrequencyA).unwrap();
    assert_eq!(r, CatResponse::FrequencyHz(14_070_000));
}
#[test] fn decode_signal_strength_normalises() {
    let r = decode("SM00015;", &CatCommand::GetSignalStrength).unwrap();
    assert_eq!(r, CatResponse::SignalStrength(0.5)); // 15/30 = 0.5
}
```

### A2: RF power control

Add `SetTxPower` / `GetTxPower` to `CatCommand`, encode/decode, wire into
`RadioControl` trait and `Ft991aRadio`.

**New config field** (`domain/config.rs`):
```rust
#[serde(default = "default_tx_power")]
pub tx_power_watts: u32,   // default 25
```

**TX guard** (`commands/tx.rs`): call `set_tx_power(config.tx_power_watts)`
before `ptt_on`. Non-fatal — log warn if it fails, proceed with TX.

**Settings dialog** (`settings-dialog.ts`): numeric TX power field (1–100 W)
on the Radio tab.

### A3: Auto-verify DATA mode before TX

Before PTT, call `get_mode()` and if not in DATA-USB/DATA-LSB, call
`set_mode()`. Correct sideband based on frequency (DATA-LSB below 10 MHz,
DATA-USB above).

---

## Session B: Signal Metering + Status Sync

### B1: S-meter via `SM0;`

Add `GetSignalStrength` to `CatCommand`. `decode` normalises the 0–30 scale
to 0.0–1.0. Emit `"signal-strength"` event every ~500ms while connected
(polled from the radio command thread, not a new thread — reuses the audio
thread's cadence).

Frontend: numeric S-meter readout in status bar (e.g. "S7" computed from
normalised value × 9, displayed as "S-meter: S7").

### B2: Comprehensive status via `IF;`

`IF;` returns a 37-character status block: VFO-A freq, RIT offset, RIT/XIT
on/off, mode, split state, and more. Use on connect instead of separate
`FA;` + `MD0;` queries.

`decode` for `IF` returns `CatResponse::Status(RadioStatus)` where:
```rust
pub struct RadioStatus {
    pub frequency_hz: u64,
    pub mode: String,
    pub is_transmitting: bool,
    pub rit_offset_hz: i32,
    pub rit_enabled: bool,
    pub split: bool,
}
```

---

## Session C: Advanced Operation

### C1: Split / VFO-B

- `GetFrequencyB` / `SetFrequencyB` → `FB;` / `FB{hz:011};`
- `GetSplit` / `SetSplit(bool)` → `FT;` / `FT0;` / `FT1;`

UI: split toggle in sidebar; VFO-B frequency input (same band-select +
freq-input pattern as VFO-A) appears when split is enabled.

### C2: RIT

- `SetRit(bool)` → `RT0;` / `RT1;`
- `ClearRit` → `RC;`
- `RitDown(u32)` / `RitUp(u32)` → `RD{n:04};` / `RU{n:04};` (units of 10 Hz)

UI: RIT offset display + up/down nudge buttons, clear button.

### C3: IF Controls

- `SetIfShift(i32)` → `IS0{+/-}{hz:04};`
- `SetIfWidth(u32)` → `SH0{code:02};` (code table varies by mode — needs lookup)
- `SetNarrow(bool)` → `NA00;` / `NA01;`

---

## File Changes Summary

| File | Change |
|------|--------|
| `src-tauri/src/cat/mod.rs` | **New** — `CatCommand`, `CatResponse` enums |
| `src-tauri/src/cat/encode.rs` | **New** — `encode()` pure function |
| `src-tauri/src/cat/decode.rs` | **New** — `decode()` pure function |
| `src-tauri/src/cat/session.rs` | **New** — `CatSession` (serial I/O only) |
| `src-tauri/src/adapters/ft991a.rs` | **Refactor** — delegate to `CatSession` |
| `src-tauri/src/ports/radio.rs` | **Extend** — new trait methods |
| `src-tauri/src/adapters/mock_radio.rs` | **Extend** — implement new methods |
| `src-tauri/src/domain/config.rs` | **Extend** — `tx_power_watts` field |
| `src-tauri/src/domain/types.rs` | **Extend** — `RadioStatus` struct |
| `src-tauri/src/commands/tx.rs` | **Extend** — TX power guard + mode guard |
| `src-tauri/src/lib.rs` | **Extend** — register `cat` module |
| `src/components/settings-dialog.ts` | **Extend** — TX power field on Radio tab |
| `src/components/status-bar.ts` | **Extend** — S-meter display |
| `tests/e2e/` | **Extend** — tests for new UI elements |
| `src-tauri/tests/cat_integration.rs` | **Extend** — integration tests for new commands |

---

## Testing Strategy

### `cat/encode.rs` and `cat/decode.rs` — zero I/O, fast, exhaustive
- One `encode` test per `CatCommand` variant
- One `decode` test per response format
- `decode("?", &any_cmd)` → `Err` for every command variant
- Edge cases: zero power (`PC000;`), max S-meter (`SM00030;`), IF boundary
  values, `IF;` response parsing

### `CatSession` — MockSerial, tests transport behavior only
- `?` response propagated as `Err`
- 50ms inter-command delay enforced
- Partial reads reassembled correctly
- TX/RX debug logs emitted

### `Ft991aRadio` — `make_radio()` helper (existing pattern)
- Wire string tests now live in `encode.rs` (simpler)
- `Ft991aRadio` tests focus on: correct `CatCommand` variant selected, correct
  `CatResponse` variant unpacked, error propagation

### Integration tests (`cat_integration.rs`)
- Extend existing tests for new commands
- TX power guard: `PC` appears before `TX1;` in MockSerial log

---

## Status

- [x] Session A: Foundation — `cat/` module, RF power, mode guard
- [x] Session B: Signal metering + status sync
- [ ] Session C: Advanced operation (split, RIT, IF filter)
