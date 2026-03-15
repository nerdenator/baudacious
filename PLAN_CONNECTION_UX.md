# Plan: Connection UX Improvements

## Status

- [x] C1 — Smart serial port detection (label known radios in dropdown)
- [x] C2 — Audio streaming indicator (explicit "audio flowing" feedback)

---

## Background

Two gaps surfaced during first hardware test prep:

1. The serial port dropdown lists every port on the system with no indication which one is
   the radio. The FT-991A presents two CP210x virtual COM ports; there's no hint which to
   pick.

2. Beyond relay clicks in the radio, there's no clear visual confirmation that serial CAT
   commands are succeeding or that audio is actually streaming into the app.

---

## C1 — Smart Serial Port Detection

### Problem

`list_serial_ports` already parses USB VID/PID (e.g., `USB (10C4:EA60)`) but displays it
as a raw hex string. The user has to know that `10C4:EA60` is a CP210x, which is what the
FT-991A uses.

### Fix

**Rust — `src-tauri/src/adapters/serial_port.rs`**

Add a static lookup table of known VID:PID pairs and extend `SerialPortInfo` with an
optional `device_hint` field:

```rust
fn known_device(vid: u16, pid: u16) -> Option<&'static str> {
    match (vid, pid) {
        (0x10C4, 0xEA60) => Some("Yaesu FT-991A / CP210x"),
        (0x0403, 0x6001) => Some("FTDI USB Serial"),
        (0x0403, 0x6015) => Some("FTDI USB Serial"),
        (0x067B, 0x2303) => Some("Prolific USB Serial"),
        _ => None,
    }
}
```

Add `device_hint: Option<String>` to `SerialPortInfo` in `src-tauri/src/domain/types.rs`.
Populate it in `list_ports()` from the lookup table.

**TypeScript — `src/types/index.ts`**

Add `deviceHint?: string` to the `SerialPortInfo` interface.

**TypeScript — `src/components/serial-panel.ts`**

Update `populateDropdown` to use the hint when available:

```typescript
option.textContent = port.deviceHint
  ? `${port.name} — ${port.deviceHint}`
  : `${port.name} (${port.portType})`;
```

If a port has a known device hint, add a CSS class (`option.classList.add('port-known')`)
so it can be visually distinguished (e.g., slightly brighter text).

**Auto-select**: If exactly one port matches a known radio VID/PID and the dropdown is
currently empty/default, auto-select it. If multiple match, do not auto-select — let the
user choose.

### Verification

- `cargo test` passes
- `npm run build` passes
- Dropdown shows `"/dev/cu.usbserial-1420 — Yaesu FT-991A / CP210x"` for FT-991A ports
- Unknown ports still show the raw VID:PID string (no regression)
- E2E tests updated to cover the hint display path

---

## C2 — Audio Streaming Indicator

### Problem

When audio starts, the only feedback is:
- The waterfall begins scrolling (only visible if there's RF signal or noise)
- The status bar shows the device name

There's no explicit "audio is streaming" indicator. In a quiet shack the waterfall can
appear frozen even when working correctly.

### Fix

Add a small streaming indicator to the audio section of the sidebar — a pulsing dot (CSS
animation) that is active while audio input is running, idle otherwise.

**HTML — `index.html`**

In the audio input row of the sidebar connections section, add:
```html
<span id="audio-stream-dot" class="stream-dot" title="Audio input streaming"></span>
```

**CSS**

```css
.stream-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--color-muted);
  margin-left: 6px;
  vertical-align: middle;
}

.stream-dot.active {
  background: var(--color-accent);
  animation: pulse 1.5s ease-in-out infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50%       { opacity: 0.35; }
}
```

**TypeScript — `src/services/audio-bridge.ts`**

The `listenAudioStatus` callback already fires with `"streaming"` / `"error:"` / `"stopped"`
status strings. Wire the dot to those events:

```typescript
export function listenAudioStreamDot(dotEl: HTMLElement): void {
  listenAudioStatus((status) => {
    dotEl.classList.toggle('active', status === 'streaming');
  });
}
```

Call `listenAudioStreamDot` from `main.ts` after the audio panel is set up, passing the
dot element.

**Backend confirmation**: `start_audio_stream` already emits `"audio-status: streaming"`
when the audio thread is up and the ring buffer is filling. No backend changes needed.

### Verification

- `npm run build` passes
- `npx playwright test` passes
- Dot is inactive before audio starts
- Dot pulses after `start_audio_stream` succeeds
- Dot goes inactive after `stop_audio_stream` or audio device lost

---

## Commit Strategy

| # | Covers | Files |
|---|--------|-------|
| 1 | C1 | `domain/types.rs`, `adapters/serial_port.rs`, `src/types/index.ts`, `src/components/serial-panel.ts` |
| 2 | C2 | `index.html`, `src/styles/` (or inline CSS), `src/services/audio-bridge.ts`, `src/main.ts` |

---

## Out of Scope

- **Carrier frequency in status bar** (hardcoded "1500 Hz") — polish item, not blocking for testing
- **TX audio pre-check** (test tone button) — useful but out of scope for this plan
- **Consolidating sidebar vs. status bar indicators** — existing indicators work; dedup is a
  cosmetic refactor
- **Additional VID:PID entries** — the table can grow over time as other radios are tested;
  start with FT-991A only
