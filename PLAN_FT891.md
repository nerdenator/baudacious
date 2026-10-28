# PLAN_FT891.md — Yaesu FT-891 Support

Focused, implementation-ready plan for "Session A" of `PLAN_RADIO_SUPPORT.md`.
FT-817/818 (Sessions B+C) are **out of scope** here.

## Goal

Let the user pick **FT-891** as the radio type, connect over USB CAT, and run PSK-31
(PTT, frequency, mode, TX power, S-meter) exactly as with the FT-991A.

## Findings from the current code (corrections to PLAN_RADIO_SUPPORT.md)

- The `cat/` module already exists (`session.rs`, `encode.rs`, `decode.rs`, `mod.rs`) and is
  ASCII/`;`-terminated — the FT-891 reuses it unchanged.
- `radio_type` already exists in `Configuration` (TS `types/index.ts:24`, settings dialog
  dropdown at `settings-dialog.ts:123`, saved at `:355`) but is **never sent to the backend**:
  `connectSerial(port, baudRate)` → `connect_serial` hardwires `Ft991aRadio`
  (`commands/serial.rs:109`). Also `main.ts:102` hardcodes `'FT-991A'` for the default config.
- `connectFromConfig(port, baud)` has 4 callers (`main.ts` ×3, `settings-dialog.ts:162`) — all
  must pass the radio type.
- Band validation (`AMATEUR_BANDS_HZ`, `is_amateur_frequency`, `band_select_code`) is private to
  `ft991a.rs`; the frontend `BAND_PLAN` in `serial-panel.ts` is a separate copy.
- `get_status()` on `Ft991aRadio` uses `IF;`.

## Items to VERIFY against the FT-891 CAT Operation Reference Manual before coding

The older plan states these as fact; I could not confirm them from the repo, so treat as open:

1. **Does the FT-891 support `IF;`?** The old plan says no (use `FA;`+`MD0;`+`TX;`). My recollection
   is that the manual does list `IF;`, but with a different body layout. Decide from the manual:
   if `IF;` works, reuse/adapt `parse_status`; otherwise compose from 3 queries.
2. **Default CAT baud.** Old plan says 9600; FT-891 menu 05-06 CAT RATE factory default may be 4800.
   Settings dialog baud list currently starts at 9600 — add 4800 if needed.
3. **Mode codes for `MD0`** (DATA-USB = `C`, DATA-LSB = `8`, etc.) — check vs `MODE_TABLE`.
4. **`BS` band codes** (0–10 match 991A? 60m = 2?) and **`PC` range** (5–100 W; 5–50 W on battery?).
5. **`TX;` response format** (`TX0;`/`TX1;`/`TX2;`?).
6. Whether CAT USB shows as one or two serial ports (Enhanced/Standard COM) — affects docs only.

Deliverable of Step 0: a short "FT-891 CAT facts" table pasted into this file.

## Design (hexagonal)

- New adapter `src-tauri/src/adapters/ft891.rs` implementing `RadioControl` over `CatSession`.
  No port-trait changes.
- Extract shared band logic so the two adapters don't duplicate tables:
  new `domain/bands.rs` with a `Band` table (name, min/max Hz, band-select code, `radios`
  availability) plus `is_amateur_frequency(hz, radio_caps)` and `band_select_code(hz)`.
  `ft991a.rs` is refactored to use it (behaviour-preserving; its existing tests must stay green).
- New `domain::RadioType` enum (`Ft991a`, `Ft891`) with `FromStr` ("FT-991A"/"FT-891"),
  `default_baud()`, and `supports_band()`. Unknown string → error (not silent FT-991A fallback).
- `connect_serial` takes `radio_type: String`, parses to `RadioType`, dispatches in a small
  factory fn `build_radio(radio_type, connection) -> Box<dyn RadioControl>` (testable without Tauri).

## Implementation steps

0. **Verify CAT facts** (above); record in this file.
1. `domain/bands.rs` + `RadioType`; refactor `ft991a.rs` onto it. Tests first/green.
2. Add `CatCommand::GetTxState` / `CatResponse::TxState(bool)` (+ encode/decode) **only if** the
   manual shows `IF;` is unusable. Otherwise adapt status parsing instead.
3. `adapters/ft891.rs`: ptt, freq (HF+6m only, 60m allowed), mode, tx power, S-meter, `get_status`,
   `Drop` PTT-release retry (copy 991A pattern), band-select-skip-if-same-band behaviour.
4. `commands/serial.rs`: `radio_type` param + `build_radio`; `MOCK_RADIO` path unchanged.
5. Frontend plumbing: `backend-api.ts` `connectSerial(port, baud, radioType)`;
   `connectFromConfig(port, baud, radioType)` and all 4 callers; `main.ts:102` default stays FT-991A.
6. Settings dialog: add `FT-891` to radio list; on radio change set baud to that radio's default
   (don't override if the user already picked a non-default baud for this profile — decide: only
   auto-set when changing radio type interactively).
7. Band selector filtered by radio type in `serial-panel.ts`: FT-891 hides 2m/70cm. If the
   connected radio reports a frequency outside its plan, fall back to "—" as today.
   (60m is not in `BAND_PLAN` today — adding it is a separate, optional follow-up.)
8. TX power slider: confirm FT-891 max (100 W; check battery/13.8 V behaviour). No UI change if 100.
9. Docs: update `CLAUDE.md` (target radios), `PLAN_RADIO_SUPPORT.md` status, `MANUAL_TEST_PLAN.md`
   (FT-891 on-air checklist: connect, set band, PTT keying with dummy load, power set/get).

## Tests (required)

### Rust unit tests
- `domain/bands.rs`: band edges per radio; 2m/70cm rejected for FT-891, accepted for FT-991A;
  60m accepted for both; between-band gaps rejected; `band_select_code` all bands.
- `RadioType`: parse known/unknown strings, `default_baud`, round-trip with config strings.
- `ft891.rs` (reuse existing `MockSerial` pattern from `ft991a.rs`; reuse `MockRadio` where possible —
  inline mocks hurt coverage):
  - ptt_on → `TX1;`, ptt_off → `TX0;`, `is_transmitting` tracks state
  - set_frequency sends `BS` then `FA`; same-band skips `BS`; band change sends `BS`
  - set_frequency rejects 2m, 70cm, non-amateur **before** sending anything
  - get/set mode (`MD0`), DATA-USB/DATA-LSB mapping
  - get/set tx power (`PC`), reject >100 W
  - get_signal_strength range 0.0–1.0 (min/max/mid)
  - get_status happy path + TX flag, and every "unexpected response → Err" branch
  - Drop while transmitting sends PTT off; Drop while idle sends nothing
- `cat/encode.rs`/`decode.rs`: new TxState cases if added (incl. echoed `TX;TX0;`, invalid input).
- `commands/serial.rs`: `build_radio` returns the right adapter for each `RadioType`; unknown
  type → `Err`; `connect_serial_inner` still corrects wrong mode on connect for an FT-891 mock.
- Coverage gate: must stay ≥ 80% (CI `coverage.yml`).

### Playwright E2E (`tests/e2e/`)
- Radio dropdown lists FT-991A and FT-891.
- Selecting FT-891 sets the suggested baud; switching back restores FT-991A's.
- `connect_serial` invoke receives `radioType: 'FT-891'` (assert via mock args) from both
  Test Connection and startup auto-connect.
- FT-891 connected: band dropdown has no 2m/70cm; FT-991A: has 6m/2m/70cm.
- Update every existing `connect_serial` mock that asserts args (`settings-radio`, `startup`, `serial` specs).
- Update visual snapshots only if the settings dialog layout changes.

### Manual (on hardware)
- Add FT-891 section to `MANUAL_TEST_PLAN.md`; dummy-load first (per `PLAN_TX_TESTING.md`).

## Risks

| Risk | Mitigation |
|---|---|
| CAT facts in old plan wrong (`IF;`, baud) | Step 0 gate before any code |
| Refactoring `ft991a.rs` band code regresses it | Behaviour-preserving; existing 40+ tests are the safety net |
| Unknown `radio_type` silently using FT-991A could key the wrong protocol | Explicit parse error |
| I have no FT-891 hardware here | Unit tests via mock serial; on-air verification is the user's |

## Out of scope
FT-817/818 binary protocol, `BinaryCatSession`, TX-power "manual" panel mode, 60m dropdown entry,
FT-710/other radios.
