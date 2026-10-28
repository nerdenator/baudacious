# PLAN_RADIO_SUPPORT.md — Multi-Radio Support: FT-891 and FT-817/818/818ND

> **FT-891 (Session A) is superseded by `PLAN_FT891.md`**, which corrects this plan's CAT assumptions
> (the FT-891 *does* support `IF;`, but it has no TX flag; `SM0;` is 3-digit; no 60m band-select code;
> TX power is 5–100 W). The FT-817/818 sections (Sessions B+C) below are still the plan of record.

## Overview

This plan adds support for the Yaesu FT-891 and the FT-817/818/818ND family to Baudacious. The FT-891 uses a near-identical ASCII CAT protocol to the FT-991A and requires a thin new adapter. The FT-817/818/818ND uses an entirely different binary (5-byte) CAT protocol that needs a dedicated encode/decode layer.

The hexagonal architecture makes this tractable: both new radios implement the existing `RadioControl` port trait, and the Tauri command layer only needs one new argument (`radio_type`) to select which adapter to instantiate.

---

## 1. Architecture Analysis

### 1.1 What the Existing CAT Layer Assumes

The current `cat/` module is completely FT-991A-specific in subtle ways:

- `CatSession` reads until a `;` byte. The FT-817 binary protocol has no such terminator — it uses fixed-length 5-byte frames. **`CatSession` cannot be reused for the FT-817.**
- `cat/encode.rs` and `cat/decode.rs` are pure ASCII functions tied to the `CatCommand` enum, which only describes FT-991A commands.
- `MODE_TABLE` in `cat/mod.rs` is an FT-991A code map. The FT-817 has a different mode byte numbering. The FT-891 shares the same codes as the FT-991A for the HF modes we care about, but adds a few differences.

### 1.2 FT-891 vs FT-991A: Protocol Differences

The FT-891 is close enough to the FT-991A that it can reuse `CatSession` (ASCII, semicolon-terminated) and the same `CatCommand` enum. The differences:

| Feature | FT-991A | FT-891 |
|---|---|---|
| Default baud | 38400 | 9600 |
| Get status | `IF;` (returns 37 or 25 chars) | No `IF;` — use `FA;` + `MD0;` + `TX;` separately |
| Band select | `BS{nn};` supported | `BS{nn};` supported with different upper-band codes (no 2m/70cm) |
| TX power | `PC{nnn};` 0–100 W | `PC{nnn};` 0–100 W (same) |
| S-meter | `SM0;` | `SM0;` (same) |
| PTT | `TX1;` / `TX0;` | `TX1;` / `TX0;` (same) |
| Mode codes | `MD0{c};` with full table | Same codes for the HF subset |
| TX state query | `TX;` returns `TX0;` or `TX1;` | Same |

The FT-891 does not have VHF/UHF, so band codes 12 (2m) and 13 (70cm) do not apply. The FT-891 band code mapping for HF is confirmed to match the FT-991A (codes 0–10 for 160m–6m).

The critical missing piece for the FT-891 is `get_status()`. The FT-991A adapter uses `IF;` for a combined frequency+mode+TX status poll. The FT-891 does not implement `IF;`. The FT-891 adapter must implement `get_status()` using three separate queries: `FA;` (frequency), `MD0;` (mode), and `TX;` (PTT state).

### 1.3 FT-817/818/818ND: Binary Protocol

The FT-817 CAT protocol is fundamentally different:

- **Wire format**: 5-byte binary frames. Bytes 0–3 are parameters (P1–P4), byte 4 is the command opcode.
- **No ASCII, no semicolons**. `CatSession.read_until_semicolon()` is completely inapplicable.
- **Frequency format**: P1–P4 encode frequency in 8-digit packed BCD (each nibble is one decimal digit), e.g. 14.070 MHz = `0x14 0x07 0x00 0x00`.
- **Responses**: Fixed-length binary (typically 1 or 5 bytes depending on command). No string parsing.
- **Mode**: Embedded as a single byte in the read-freq response, with its own mode table.

Key FT-817 opcodes:

| Opcode | Command | Params | Response |
|---|---|---|---|
| `0x01` | Set VFO-A frequency | P1–P4 = BCD freq | 1 byte (0x00 = OK) |
| `0x03` | Read VFO-A frequency + mode | P1–P4 = 0 | 5 bytes: [BCD freq x4][mode byte] |
| `0x08` | PTT on | P1–P4 = 0 | 1 byte (0x00 = OK) |
| `0x88` | PTT off | P1–P4 = 0 | 1 byte (0x00 = OK) |
| `0x07` | Set operating mode | P1 = mode byte, P2–P4 = 0 | 1 byte (0x00 = OK) |
| `0xE7` | Read S-meter | P1–P4 = 0 | 5 bytes: [S-meter][flags x4] |

FT-817 mode byte table (differs from FT-991A mode codes):

| Byte | Mode |
|---|---|
| `0x00` | LSB |
| `0x01` | USB |
| `0x02` | CW |
| `0x03` | CW-R |
| `0x04` | AM |
| `0x08` | FM |
| `0x0A` | DATA-LSB (PKT-LSB) |
| `0x0C` | DATA-USB (PKT-USB) |
| `0x0E` | DATA-FM (PKT-FM) |

TX power is **not controllable via CAT** on the FT-817/818. Power is set by the physical `PWR` knob only. The `set_tx_power()` and `get_tx_power()` methods return a descriptive error.

### 1.4 Session Architecture

`CatSession` hardwires the "ASCII, read until `;`" I/O pattern. For the FT-817 we need "binary, read exactly N bytes".

**Decision: Separate `BinaryCatSession` struct.** Create `src-tauri/src/cat/binary_session.rs` alongside the existing `session.rs`. `Ft817Radio` owns a `BinaryCatSession` directly. No changes to `CatSession`.

This keeps the existing FT-991A code completely unchanged and gives the FT-817 a clean home with purpose-built I/O logic.

### 1.5 Radio Type Selection

Currently `connect_serial` in `commands/serial.rs` hardwires `Ft991aRadio::new(connection)`. A `radio_type: String` parameter selects the adapter. `Configuration.radio_type` already exists with default `"FT-991A"` — no migration needed.

---

## 2. New Files to Create

### Rust backend

```
src-tauri/src/adapters/ft891.rs          — FT-891 RadioControl adapter
src-tauri/src/adapters/ft817.rs          — FT-817/818/818ND RadioControl adapter
src-tauri/src/cat/binary_session.rs      — Binary frame I/O session for FT-817
src-tauri/src/cat/ft817_encode.rs        — Pure: Ft817Command → [u8; 5] (no I/O)
src-tauri/src/cat/ft817_decode.rs        — Pure: [u8] → Ft817Response (no I/O)
```

### Frontend

No new files needed. Changes to existing files:
- `src/components/settings-dialog.ts` — add FT-891 and FT-817 to radio type dropdown; baud auto-suggest; show/hide TX power panel for FT-817
- `src/components/serial-panel.ts` — pass `radio_type` to `connectSerial`; per-radio band plan filtering
- `src/services/backend-api.ts` — update `connectSerial` signature to include `radioType`
- `src/components/tx-power-panel.ts` — add `setTxPowerPanelMode('cat' | 'manual')`

---

## 3. Detailed Implementation

### 3.1 Phase A: FT-891 Adapter

#### CAT protocol additions (`cat/mod.rs`, `cat/encode.rs`, `cat/decode.rs`)

Add `CatCommand::GetTxState` and `CatResponse::TxState(bool)` for FT-891's `TX;` query:

```rust
// cat/mod.rs
GetTxState,            // → "TX;" — returns TX0; or TX1;
TxState(bool),         // response: false = RX, true = TX
```

```rust
// cat/encode.rs
GetTxState => "TX;".into(),

// cat/decode.rs
GetTxState => parse_tx_state(response),

fn parse_tx_state(response: &str) -> Psk31Result<CatResponse> {
    let trimmed = response.trim_end_matches(';');
    // Strip command echo (some USB adapters echo TX;TX0;)
    let trimmed = trimmed.strip_prefix("TX").unwrap_or(trimmed);
    match trimmed {
        "0" | "TX0" => Ok(CatResponse::TxState(false)),
        "1" | "TX1" => Ok(CatResponse::TxState(true)),
        _ => Err(Psk31Error::Cat(format!("Invalid TX state response: '{response}'"))),
    }
}
```

#### Shared band validation (`domain/frequency.rs`)

Extract the HF-only and HF+VHF variants so both adapters share the same list without duplication:

```rust
/// FT-891 and FT-817 valid bands: HF (160m–10m) + 6m + 2m
/// (FT-817 has 2m; FT-891 has 6m but no 2m/70cm — use the larger superset and
/// let each adapter enforce its own upper limit via band_select_code)
pub const AMATEUR_BANDS_HF: &[(u64, u64)] = &[
    (1_800_000,   2_000_000),   // 160m
    (3_500_000,   4_000_000),   // 80m
    (5_332_000,   5_405_000),   // 60m
    (7_000_000,   7_300_000),   // 40m
    (10_100_000,  10_150_000),  // 30m
    (14_000_000,  14_350_000),  // 20m
    (18_068_000,  18_168_000),  // 17m
    (21_000_000,  21_450_000),  // 15m
    (24_890_000,  24_990_000),  // 12m
    (28_000_000,  29_700_000),  // 10m
    (50_000_000,  54_000_000),  // 6m
];

pub fn is_amateur_frequency_hf(hz: u64) -> bool {
    AMATEUR_BANDS_HF.iter().any(|&(lo, hi)| hz >= lo && hz <= hi)
}
```

#### `src-tauri/src/adapters/ft891.rs`

```rust
pub struct Ft891Radio {
    session: CatSession,
    is_transmitting: bool,
    last_band_code: Option<u8>,
}

impl Ft891Radio {
    pub fn new(serial: Box<dyn SerialConnection>) -> Self { ... }
}
```

Key differences from `Ft991aRadio`:
- `set_frequency()` validates with `is_amateur_frequency_hf()` (no 2m/70cm)
- `band_select_code()` is HF+6m only (codes 0–10, no 12/13)
- `get_status()` uses three queries: `FA;`, `MD0;`, `TX;`
- `get_tx_state()` calls `execute(&CatCommand::GetTxState)` → `CatResponse::TxState`
- `Drop` impl: same PTT-release-on-drop retry pattern as `Ft991aRadio`

#### `commands/serial.rs` — radio type dispatch

```rust
#[tauri::command]
pub fn connect_serial(
    state: State<AppState>,
    port: String,
    baud_rate: u32,
    radio_type: String,
) -> Result<RadioInfo, String> {
    // ...
    let radio: Box<dyn RadioControl> = match radio_type.as_str() {
        "FT-891"             => Box::new(Ft891Radio::new(connection)),
        "FT-817" | "FT-818" => Box::new(Ft817Radio::new(connection)),
        _                    => Box::new(Ft991aRadio::new(connection)),
    };
```

### 3.2 Phase B: FT-817 Binary Layer

#### `src-tauri/src/cat/binary_session.rs`

```rust
const COMMAND_DELAY_MS: u64 = 10;   // FT-817 manual recommends ≥10ms
const PORT_SETTLE_MS: u64 = 200;

pub struct BinaryCatSession {
    serial: Box<dyn SerialConnection>,
    last_command_time: Option<Instant>,
}

impl BinaryCatSession {
    pub fn new(serial: Box<dyn SerialConnection>) -> Self {
        std::thread::sleep(Duration::from_millis(PORT_SETTLE_MS));
        Self { serial, last_command_time: None }
    }

    /// Send a 5-byte frame, read exactly `response_len` bytes.
    pub fn execute(&mut self, frame: [u8; 5], response_len: usize) -> Psk31Result<Vec<u8>> { ... }

    fn read_exact(&mut self, n: usize) -> Psk31Result<Vec<u8>> { ... }
    fn ensure_command_delay(&self) { ... }
}
```

#### `src-tauri/src/cat/ft817_encode.rs`

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Ft817Command {
    SetFrequency(u64),
    GetFrequencyAndMode,
    PttOn,
    PttOff,
    SetMode(u8),          // mode byte (use FT817_MODE_TABLE to look up)
    GetSignalStrength,
}

pub const FT817_MODE_TABLE: &[(u8, &str)] = &[
    (0x00, "LSB"),
    (0x01, "USB"),
    (0x02, "CW"),
    (0x03, "CW-R"),
    (0x04, "AM"),
    (0x08, "FM"),
    (0x0A, "DATA-LSB"),
    (0x0C, "DATA-USB"),
    (0x0E, "DATA-FM"),
];

pub fn encode(cmd: &Ft817Command) -> [u8; 5] { ... }

/// Pack frequency in Hz into 4 bytes of 8-digit packed BCD.
/// Example: 14_070_000 Hz → [0x14, 0x07, 0x00, 0x00]
fn encode_bcd(hz: u64) -> [u8; 4] { ... }
```

#### `src-tauri/src/cat/ft817_decode.rs`

```rust
/// Decode 5-byte GetFrequencyAndMode response → (hz, mode_string)
pub fn decode_frequency_and_mode(bytes: &[u8]) -> Psk31Result<(u64, String)> { ... }

/// Decode 5-byte S-meter response → signal strength 0.0..=1.0
pub fn decode_smeter(bytes: &[u8]) -> Psk31Result<f32> { ... }

fn decode_bcd(b: &[u8]) -> u64 { ... }

fn decode_mode(byte: u8) -> Psk31Result<String> {
    FT817_MODE_TABLE.iter()
        .find(|&&(b, _)| b == byte)
        .map(|&(_, name)| name.to_string())
        .ok_or_else(|| Psk31Error::Cat(format!("Unknown FT-817 mode byte: 0x{byte:02X}")))
}
```

#### `src-tauri/src/adapters/ft817.rs`

```rust
pub struct Ft817Radio {
    session: BinaryCatSession,
    is_transmitting: bool,
}

impl RadioControl for Ft817Radio {
    fn ptt_on(&mut self) -> Psk31Result<()> { ... }    // opcode 0x08, read 1 ack byte
    fn ptt_off(&mut self) -> Psk31Result<()> { ... }   // opcode 0x88, read 1 ack byte
    fn get_frequency(&mut self) -> Psk31Result<Frequency> { ... }  // opcode 0x03, read 5 bytes
    fn set_frequency(&mut self, freq: Frequency) -> Psk31Result<()> { ... }  // opcode 0x01
    fn get_mode(&mut self) -> Psk31Result<String> { ... }  // opcode 0x03, extract mode byte
    fn set_mode(&mut self, mode: &str) -> Psk31Result<()> { ... }  // opcode 0x07
    fn get_tx_power(&mut self) -> Psk31Result<u32> {
        Err(Psk31Error::Cat("FT-817 TX power is not controllable via CAT".into()))
    }
    fn set_tx_power(&mut self, _watts: u32) -> Psk31Result<()> {
        Err(Psk31Error::Cat("FT-817 TX power is set by the front-panel PWR knob only".into()))
    }
    fn get_signal_strength(&mut self) -> Psk31Result<f32> { ... }  // opcode 0xE7
    fn get_status(&mut self) -> Psk31Result<RadioStatus> { ... }   // opcode 0x03 + rit=0/split=false
    fn is_transmitting(&self) -> bool { self.is_transmitting }
}
```

FT-817 band validation: HF (160m–10m) + 6m + 2m. The FT-817 does not have a `BS;` command — frequency is set directly without a band-select step.

### 3.3 Phase C: Frontend

#### `backend-api.ts`

```typescript
export async function connectSerial(
  port: string,
  baudRate: number,
  radioType: string,
): Promise<RadioInfo> {
  return invoke('connect_serial', { port, baudRate, radioType });
}
```

#### `settings-dialog.ts`

```typescript
const RADIO_MODELS = [
  { value: 'FT-991A', label: 'Yaesu FT-991A',          defaultBaud: 38400 },
  { value: 'FT-891',  label: 'Yaesu FT-891',            defaultBaud:  9600 },
  { value: 'FT-817',  label: 'Yaesu FT-817/818/818ND',  defaultBaud:  9600 },
];
// Populate dropdown; on change, update baudSelect default
```

#### `tx-power-panel.ts`

```typescript
export function setTxPowerPanelMode(mode: 'cat' | 'manual'): void {
  // mode === 'cat':    show slider (default)
  // mode === 'manual': hide slider, show "Set via front-panel PWR knob" label
}
```

#### `serial-panel.ts`

Band plan filtered per radio type at `handleConnectSuccess`:
- FT-991A: full plan (160m–70cm)
- FT-891: HF + 6m (160m–6m, no 2m/70cm)
- FT-817/818: HF + 6m + 2m (160m–2m, no 70cm)

---

## 4. Tests

### 4.1 Unit Tests — `cat/ft817_encode.rs`

```
encode_set_frequency_20m()       → [0x14,0x07,0x00,0x00,0x01]
encode_set_frequency_160m()      → [0x01,0x80,0x00,0x00,0x01]
encode_ptt_on()                  → [0x00,0x00,0x00,0x00,0x08]
encode_ptt_off()                 → [0x00,0x00,0x00,0x00,0x88]
encode_set_mode_data_usb()       → [0x0C,0x00,0x00,0x00,0x07]
encode_bcd_14070000_hz()         → [0x14,0x07,0x00,0x00]
encode_bcd_7035000_hz()          → [0x07,0x03,0x50,0x00]
encode_bcd_roundtrip()           → decode(encode(f)) == f for all sample freqs
```

### 4.2 Unit Tests — `cat/ft817_decode.rs`

```
decode_freq_mode_20m_data_usb()  → (14_070_000, "DATA-USB")
decode_freq_mode_40m_data_lsb()  → (7_035_000, "DATA-LSB")
decode_mode_all_known_bytes()    → each FT817_MODE_TABLE entry
decode_mode_unknown_byte()       → Err
decode_smeter_zero()             → 0.0
decode_smeter_full_scale()       → ~1.0 (clamped)
decode_bcd_roundtrip()           → encode then decode
```

### 4.3 Unit Tests — `cat/encode.rs` + `cat/decode.rs` additions

```
encode_get_tx_state()            → "TX;"
decode_tx_state_tx0()            → TxState(false)
decode_tx_state_tx1()            → TxState(true)
decode_tx_state_with_echo()      → TxState(false) from "TX;TX0;"
decode_tx_state_invalid()        → Err
```

### 4.4 Unit Tests — `adapters/ft891.rs`

Using the existing `MockSerial` pattern:

```
ptt_on_sends_tx1()
ptt_off_sends_tx0()
get_status_issues_three_commands()      → FA; MD0; TX; in sequence
set_frequency_hf_sends_bs_then_fa()
set_frequency_rejects_2m()             → FT-891 has no 2m
set_frequency_rejects_70cm()
set_frequency_rejects_non_amateur()
get_tx_power_sends_pc_query()
set_tx_power_100w_succeeds()
set_tx_power_101w_rejected()
drop_releases_ptt_when_transmitting()
```

### 4.5 Unit Tests — `adapters/ft817.rs`

Using a `MockBinarySerial` (returns pre-canned byte slices):

```
ptt_on_sends_correct_frame()
ptt_off_sends_correct_frame()
get_frequency_sends_0x03_and_decodes()
set_frequency_sends_bcd_frame()
set_frequency_rejects_70cm()           → FT-817 has no 70cm
set_frequency_rejects_non_amateur()
get_mode_returns_decoded_string()
set_mode_data_usb_sends_0x0c()
get_tx_power_returns_err()
set_tx_power_returns_err()
get_signal_strength_decodes_smeter()
get_status_assembles_radio_status()
drop_releases_ptt_when_transmitting()
```

### 4.6 E2E Tests — `tests/e2e/radio-support.spec.ts`

```
radio_type_dropdown_shows_all_three_models()
selecting_ft891_auto_sets_baud_9600()
selecting_ft817_auto_sets_baud_9600()
selecting_ft991a_restores_baud_38400()
ft817_connect_hides_tx_power_slider()
ft817_connect_shows_manual_tx_power_label()
ft991a_connect_shows_tx_power_slider()
ft817_band_plan_excludes_70cm()
ft891_band_plan_excludes_2m_and_70cm()
ft991a_band_plan_includes_6m_2m_70cm()
test_connection_passes_radio_type_to_backend()
```

---

## 5. Implementation Sequence

### Session A — FT-891 (est. 3–4 hours)

1. Add `CatCommand::GetTxState` + `CatResponse::TxState(bool)` to `cat/mod.rs`
2. Add encode/decode in `cat/encode.rs` + `cat/decode.rs`
3. Extract shared band list to `domain/frequency.rs`
4. Create `src-tauri/src/adapters/ft891.rs`
5. Update `connect_serial` to accept `radio_type` and dispatch
6. Update `connectSerial` in `backend-api.ts` + callers
7. Update settings dialog: FT-891 model + baud auto-suggest
8. Write all FT-891 unit tests + E2E radio type dropdown tests

### Session B — FT-817 Binary Layer (est. 4–5 hours)

1. Create `src-tauri/src/cat/binary_session.rs`
2. Create `src-tauri/src/cat/ft817_encode.rs` + `ft817_decode.rs`
3. Create `src-tauri/src/adapters/ft817.rs`
4. Add `"FT-817" | "FT-818"` arm to `connect_serial` dispatch
5. Write all FT-817 unit tests

### Session C — FT-817 Frontend UX (est. 2–3 hours)

1. Add `setTxPowerPanelMode('cat' | 'manual')` to `tx-power-panel.ts`
2. Call it from `handleConnectSuccess` based on `radio_type`
3. Add FT-817 to settings dialog radio type dropdown
4. Add per-radio-type band plan filtering in `serial-panel.ts`
5. Write E2E tests for FT-817 TX power panel + band plan behavior

---

## 6. Known Issues and Mitigations

| Issue | Mitigation |
|---|---|
| FT-817 ACK byte ambiguity: some commands return `0x00` on success, `0xF0` on error; others return no byte | `BinaryCatSession::execute()` takes `response_len` — callers specify 0 or 1 per the CAT manual |
| FT-817: mode set while transmitting not supported | Guard `set_mode` with `if self.is_transmitting { return Err(...) }` |
| FT-891 TX; echo: some firmware returns `TX;TX0;` | `parse_tx_state` strips leading `TX;` prefix before matching |
| `connect_serial` now requires `radioType` argument | All E2E test mocks of `connect_serial` must include `radioType` in the invoke args |
| FT-817 TX power: slider shown but non-functional without panel mode API | Frontend hides slider for FT-817 before any CAT call is made |

---

## 7. Key Files

| File | Role |
|---|---|
| `src-tauri/src/adapters/ft991a.rs` | Pattern to follow for FT-891 adapter |
| `src-tauri/src/cat/session.rs` | Reused unchanged by FT-891; not used by FT-817 |
| `src-tauri/src/cat/mod.rs` | Add `GetTxState`, `TxState`, declare new ft817 modules |
| `src-tauri/src/commands/serial.rs` | Single dispatch point for `radio_type` adapter selection |
| `src/components/settings-dialog.ts` | Radio type dropdown, baud auto-suggest, only place `radio_type` enters config |
| `src/components/serial-panel.ts` | `connectFromConfig`, band plan per radio type |
