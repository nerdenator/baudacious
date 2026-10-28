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

## FT-891 CAT facts (Step 0 — DONE)

Source: Yaesu *FT-891 CAT Operation Reference Book* (20 pp, 2018), copy hosted at
[rigreference.com](https://www.rigreference.com/storage/manuals/yaesu/FT-891%20CAT%20Reference%20Book.pdf--5a6a4c01735456.71645851.pdf).
Text extracted and checked against the existing `cat/` code. **Several differences from the
FT-991A break reuse as-is**, and two claims in `PLAN_RADIO_SUPPORT.md` were wrong.

| Topic | FT-891 (per manual) | Impact |
|---|---|---|
| `IF;` | **Supported** (read only). Answer body is the **25-char** layout: P1 mem ch (3), P2 freq (9), P3 clarifier ±dddd (5), P4 CLAR on (1), P5 fixed (1), P6 mode (1), P7 VFO/mem (1), P8 CTCSS (1), P9 fixed (2), P10 shift (1) | Old plan ("no IF;") was wrong. But see next row. |
| `IF;` has **no TX flag** | Existing `parse_status_compact` reads `body[21]` as TX — on the FT-891 that byte is **CTCSS** mode (0/1/2), so CTCSS on would report "transmitting". `body[24]` is simplex/repeater **shift**, which the 991A parser treats as *split*. | Needs an FT-891-specific IF parser. TX state comes from `TX;` instead. |
| `TX;` read | `TX0;` = RX, `TX1;` = CAT TX on, `TX2;` = radio TX on (front-panel PTT/mic). | Treat any non-zero as transmitting. |
| `SM0;` | Answer `SM0` + **3 digits** `000–255`. Existing decoder requires 4 digits and scales 0–30 → would return an error / wrong scale. | New decode path, scale `/255`. |
| `MD0;` | Same P2 codes as 991A for `1–9, B, C, D` (`C` = DATA-USB, `8` = DATA-LSB). `A` is unused ("-"); no C4FM (`E`). | `MODE_TABLE` is a superset; reuse it. |
| `BS` | `00` 1.8, `01` 3.5, **`02` unused (no 60m)**, `03` 7, `04` 10, `05` 14, `06` 18, `07` 21, `08` 24.5, `09` 28, `10` 50, `11` GEN, `12` MW. No 2m/70cm. | 60m: skip `BS`, set via `FA` only. |
| `FA` | 9 digits, `000030000–056000000` Hz. | Matches 991A's 9-digit write; 2m/70cm out of range. |
| `PC` | `PC` + 3 digits, **`005–100`** W (min is 5, not 0). | Reject <5 and >100 for FT-891. |
| CAT baud | Menu `05-06` CAT RATE: `0`=4800, `1`=9600, `2`=19200, `3`=38400. The reference book's menu table shows `1` (9600) in its default column; a web summary of the operating manual says 4800. **Conflicting — don't assume.** | Offer 4800/9600/19200/38400; suggest 9600 but tell the user to match the radio's menu. |
| Other menu items | `05-07` CAT TOT (10 ms–3 s), `05-08` CAT RTS (enable/disable). | Document in manual-test plan. |

Remaining unknown: exact CAT command timing/inter-command delay (not in the book). Reuse
`CatSession`'s existing delay and confirm on hardware.

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
2. CAT layer: add `CatCommand::GetTxState` (`TX;` → `CatResponse::TxState(bool)`, non-zero = TX);
   add an FT-891 `IF;` parser (freq `[3..12]`, clarifier `[12..17]`/`[17]`, mode `[19]`; **no** TX/split
   from IF) and a 3-digit `SM0` decode (`/255`). Keep the 991A parsers untouched; select by radio
   (separate `Ft891` decode fns or a `CatDialect` parameter on `CatSession` — pick the smaller diff).
3. `adapters/ft891.rs`: ptt, freq (HF+6m only; 60m allowed but **skip `BS`** — no band code), mode,
   tx power (5–100 W), S-meter, `get_status` (= `IF;` + `TX;`),
   `Drop` PTT-release retry (copy 991A pattern), band-select-skip-if-same-band behaviour.
4. `commands/serial.rs`: `radio_type` param + `build_radio`; `MOCK_RADIO` path unchanged.
5. Frontend plumbing: `backend-api.ts` `connectSerial(port, baud, radioType)`;
   `connectFromConfig(port, baud, radioType)` and all 4 callers; `main.ts:102` default stays FT-991A.
6. Settings dialog: add `FT-891` to radio list; add 4800 to the baud list; on radio change suggest 9600 for FT-891 (hint: "must match radio menu 05-06"); set baud to that radio's default
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
  - set_frequency sends `BS` then `FA`; same-band skips `BS`; band change sends `BS`; 60m sends `FA` only
  - set_frequency rejects 2m, 70cm, non-amateur **before** sending anything
  - get/set mode (`MD0`), DATA-USB/DATA-LSB mapping
  - get/set tx power (`PC`), reject <5 W and >100 W
  - get_signal_strength range 0.0–1.0 (min/max/mid)
  - get_status happy path + TX flag, and every "unexpected response → Err" branch
  - Drop while transmitting sends PTT off; Drop while idle sends nothing
- `cat/encode.rs`/`decode.rs`: `TX;` → `TX0/TX1/TX2` (and invalid); FT-891 `IF;` parse with CTCSS
  on (`P8=1`) and repeater shift (`P10=1`) asserting **not** transmitting / **not** split; clarifier
  sign/offset; short body → Err; `SM0128;` → ~0.5, `SM0000;` → 0, `SM0255;` → 1.0, 4-digit input → Err.
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
| 991A parsers misread FT-891 `IF;`/`SM0;` (CTCSS→TX, shift→split, 3-digit S-meter) | FT-891-specific parsers + tests for each |
| CAT baud default conflicting in sources | Offer all four rates; UI hint to match radio menu 05-06 |
| Refactoring `ft991a.rs` band code regresses it | Behaviour-preserving; existing 40+ tests are the safety net |
| Unknown `radio_type` silently using FT-991A could key the wrong protocol | Explicit parse error |
| Set commands may or may not get a reply on real hardware | Sent write-only; FT-891 query reads wait for the matching frame and skip strays/echo (`find_reply_frame`). A stray `?;` NAK from a failed Set surfaces as one error on the next query |
| Power < 5 W would leave the radio at its old power (TX path ignores a failed power set) | Adapter raises to 5 W; slider/number box minimum is 5 W on FT-891; stored lower value raised on connect |
| I have no FT-891 hardware here | Unit tests via mock serial; on-air verification is the user's |

## Out of scope
FT-817/818 binary protocol, `BinaryCatSession`, TX-power "manual" panel mode, 60m dropdown entry,
FT-710/other radios.
