# Plan: Fix Audio Output Device Enumeration

## Problem

The output device dropdown currently shows **all** devices (input and output) because
`host.output_devices()` and `supported_output_configs()` both silently omit the FT-991A
USB Audio CODEC on macOS/CoreAudio, even though it physically supports output.

The workaround (`host.devices()` with no filter) was too broad — input-only devices
like the MacBook microphone now appear in the output dropdown.

## Root Cause

`supported_output_configs()` returns empty for some USB duplex devices on CoreAudio.
However `device.default_output_config()` takes a different code path (asks CoreAudio for
its preferred config rather than enumerating all) and **does** succeed for these devices.

## Fix

### 1. `src-tauri/src/domain/types.rs` — `AudioDeviceInfo`

Add `is_output: bool` alongside the existing `is_input: bool`:

```rust
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_input: bool,
    pub is_output: bool,   // ← new
    pub is_default: bool,
}
```

### 2. `src-tauri/src/adapters/cpal_audio.rs` — `CpalAudioInput::list_devices`

Replace the current "all devices, no filter" output section with a capability check
using `default_output_config()` (more reliable than `supported_output_configs()` on macOS):

```rust
// Determine output-capable devices via default_output_config() — more reliable
// than supported_output_configs() on macOS CoreAudio for USB duplex devices.
let default_output_name = host.default_output_device()
    .and_then(|d| d.name().ok());

let mut devices = Vec::new();

if let Ok(all_devices) = host.devices() {
    for device in all_devices {
        let name = device.name().unwrap_or_else(|_| "Unknown".into());
        let is_input = device.default_input_config().is_ok();
        let is_output = device.default_output_config().is_ok();

        if !is_input && !is_output { continue; } // skip unusable devices

        let is_default_input = default_input_name.as_deref() == Some(&name);
        let is_default_output = default_output_name.as_deref() == Some(&name);

        devices.push(AudioDeviceInfo {
            id: name.clone(),
            name,
            is_input,
            is_output,
            is_default: is_default_input || is_default_output,
        });
    }
}
```

Remove the separate input/output enumeration loops — one pass handles both.

### 3. `src-tauri/src/adapters/cpal_audio.rs` — `CpalAudioOutput::list_devices`

Same approach — enumerate all devices, emit only those where
`device.default_output_config().is_ok()`. Set `is_input`/`is_output` correctly.

### 4. `src/types/index.ts` — TypeScript type

```typescript
export interface AudioDeviceInfo {
  id: string;
  name: string;
  isInput: boolean;
  isOutput: boolean;   // ← new
  isDefault: boolean;
}
```

### 5. `src/components/audio-panel.ts` — dropdown filter

The current filter uses `device.isInput` for the input dropdown and `!device.isInput`
for the output dropdown. Change the output filter to `device.isOutput`:

```typescript
if (device.isInput) inputDropdown.appendChild(option);
if (device.isOutput && outputDropdown) outputDropdown.appendChild(option.cloneNode(true));
```

A duplex device (USB Audio CODEC) will appear in **both** dropdowns correctly.

## Verification

- USB Audio CODEC appears in output dropdown ✓
- MacBook microphone does NOT appear in output dropdown ✓
- MacBook speakers do NOT appear in input dropdown ✓
- Duplex devices appear in both dropdowns ✓

## Files to Change

| File | Change |
|------|--------|
| `src-tauri/src/domain/types.rs` | Add `is_output: bool` to `AudioDeviceInfo` |
| `src-tauri/src/adapters/cpal_audio.rs` | Single-pass enumeration with `default_output_config()` check |
| `src/types/index.ts` | Add `isOutput: boolean` to `AudioDeviceInfo` |
| `src/components/audio-panel.ts` | Filter output dropdown by `device.isOutput` |
