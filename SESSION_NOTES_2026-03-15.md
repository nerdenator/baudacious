# Session Notes — 2026-03-15

## What We Did

### Live FT-991A Hardware Testing & CAT Fixes (commit 8857606)

Discovered and fixed several real hardware issues during live testing:

1. **`set_mode` was silently dropped** — MD0X; SET has no ack on this firmware.
   Changed `ft991a.rs` to use `execute_write_only()` for `set_mode`.

2. **Connect didn't correct mode** — BS; recalls band memory which may store a phone
   mode (e.g. LSB). `connect_serial` now calls `data_mode_for_frequency()` and
   corrects to DATA-LSB/USB if needed.

3. **IF; compact decoder read wrong frequency** — `parse_status_compact` was reading
   body[0..11] as 11-digit freq, giving ~100 MHz instead of 3.5 MHz. This radio uses
   a 3-char prefix (`001`) + 9-digit frequency at body[3..12]. Fixed decoder + updated
   the unit test that was asserting the wrong (100 MHz) value.

4. **`RadioStatus` missing `serde(rename_all = "camelCase")`** — `frequency_hz` was
   arriving as `undefined` in TypeScript, causing `detectBand()` to always return null
   and clear the band selector every poll cycle.

5. **Radio state polling** — Added `get_radio_state` command (IF;) and a 2s poll in
   `serial-panel.ts` so the UI tracks VFO/band changes made on the rig. Poll is
   suppressed for 4s after any user-initiated change to avoid overwriting UI state.

6. **SM0; throttled** from 500ms → 2000ms to reduce CAT bus noise.

### UI: Waterfall controls opaque (commit 31c2db6)

`.waterfall-header` background changed from a gradient-to-transparent to solid
`var(--bg-primary)`. Palette select and zoom buttons use `var(--bg-secondary)`.

### Audio output device enumeration (commit 4abacd2)

`host.output_devices()` and `supported_output_configs()` both silently omit the
FT-991A USB Audio CODEC on macOS/CoreAudio. Fixed by enumerating `host.devices()`
(all devices, no capability filter) in both `CpalAudioInput::list_devices` and
`CpalAudioOutput::list_devices`. `CpalAudioOutput::start` also updated to use
`host.devices()` so it can actually find and open the device.

## Where We Left Off

- USB Audio CODEC now appears in the output dropdown ✓
- **Not yet tested:** whether `build_output_stream` succeeds when TX is attempted
  with USB Audio CODEC selected as output. This is the next thing to try.
- If output stream fails, likely need to set `channels: 2` or probe supported
  configs from the device rather than hardcoding mono 48kHz.

## Known Open Items

- P7 — Encoder phase flip timing (raised cosine shaping, deferred)
- P9 — CSP disabled in tauri.conf.json (low priority, local-only app)
- TX with USB Audio CODEC: not yet tested end-to-end
