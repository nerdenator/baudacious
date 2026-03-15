# Plan: TX Power Panel (replaces PTT pane)

## Problem

1. The PTT pane (RX/TX badge + "Receiving" text) is low-value real estate.
2. `start_tx` blindly applies `tx_power_watts` from config (default 25 W), overriding
   whatever the radio was set to.
3. There is no UI to view or adjust TX power.

## Solution

Replace the PTT sidebar section with a **TX Power** pane that:
- Shows a slider with a colour-coded track (green / yellow / red zones)
- Reads the radio's current power on connect and pre-fills the slider
- Persists the chosen value to config on change (so it survives restart)
- Retains the `.ptt-indicator` / `.ptt-status` elements inside the new pane
  so `control-panel.ts` continues to work without modification

---

## Colour Zones

| Range | Colour | Meaning |
|-------|--------|---------|
| 0–25 W | Green `#22c55e` | Normal PSK-31 power |
| 25–30 W | Yellow `#eab308` | Caution |
| 30–100 W | Red `#ef4444` | High power |

The slider **track** uses a static CSS gradient across the full 0–100 W range.
The **value display** text changes colour to match the active zone.

---

## Tests

### Rust unit tests (`src-tauri/src/commands/`)

- `get_tx_power` returns value from mock radio (`get_tx_power` on mock returns 10 W → command returns `Ok(10)`)
- `set_tx_power_config` updates `AppState.config.tx_power_watts` correctly
- `set_tx_power_config` rejects watts > 100 with an error
- `set_tx_power_config` with 0 W is accepted (valid min)

### Playwright E2E tests (`tests/e2e/tx-power.spec.ts`)

- Slider renders with default value 10 and correct `tx-power-green` class on value display
- Moving slider to 28 W → value display shows `28 W` with `tx-power-yellow` class
- Moving slider to 31 W → value display shows `31 W` with `tx-power-red` class
- Moving slider calls `set_tx_power_config` invoke with correct watts argument (debounced)
- On radio connect, `get_tx_power` result (mocked to 15 W) is reflected in slider value

---

## Files to Change

### 1. `index.html` — replace PTT section

Replace:
```html
<!-- PTT Status -->
<div class="sidebar-section">
  <div class="section-label">PTT</div>
  <div class="ptt-container">
    <div class="ptt-indicator rx">RX</div>
    <div class="ptt-status">Receiving</div>
  </div>
</div>
```

With:
```html
<!-- TX Power -->
<div class="sidebar-section">
  <div class="section-label">TX Power</div>
  <div class="tx-power-container">
    <div class="tx-power-row">
      <input type="range" id="tx-power-slider" class="tx-power-slider"
             min="0" max="100" value="10" step="1">
      <span id="tx-power-value" class="tx-power-value tx-power-green">25 W</span>
    </div>
    <div class="tx-power-zones">
      <span class="tx-zone-label tx-zone-green">0–25</span>
      <span class="tx-zone-label tx-zone-yellow">25–30</span>
      <span class="tx-zone-label tx-zone-red">30–100 W</span>
    </div>
    <div class="ptt-row">
      <div class="ptt-indicator rx">RX</div>
      <div class="ptt-status">Receiving</div>
    </div>
  </div>
</div>
```

### 2. `src-tauri/src/commands/radio.rs` — add `get_tx_power` command

```rust
#[tauri::command]
pub fn get_tx_power(app: AppHandle, state: State<AppState>) -> Result<u32, String> {
    with_radio(&state, &app, |r| r.get_tx_power())
        .map_err(|e| e.to_string())
}
```

### 3. `src-tauri/src/commands/config.rs` — add `set_tx_power_config` command

Updates the in-memory config and auto-saves to disk:

```rust
#[tauri::command]
pub fn set_tx_power_config(watts: u32, state: State<AppState>) -> Result<(), String> {
    if watts > 100 {
        return Err(format!("TX power {watts} W exceeds maximum (100 W)"));
    }
    let config = {
        let mut cfg = state.config.lock().map_err(|_| "config lock poisoned")?;
        cfg.tx_power_watts = watts;
        cfg.clone()
    };
    save_config_to_disk(&config).map_err(|e| e.to_string())
}
```

`save_config_to_disk` is the same helper already used by `save_configuration` —
extract it (or call `save_configuration` internally by re-using the existing logic).

### 4. `src-tauri/src/lib.rs` — register new commands

Add `commands::radio::get_tx_power` and `commands::config::set_tx_power_config`
to the `invoke_handler!` list.

### 5. `src/services/backend-api.ts` — add two calls

```typescript
export async function getTxPower(): Promise<number> {
  return invoke<number>('get_tx_power');
}

export async function setTxPowerConfig(watts: number): Promise<void> {
  return invoke('set_tx_power_config', { watts });
}
```

### 6. `src/components/tx-power-panel.ts` — new component

```typescript
import { getTxPower, setTxPowerConfig } from '../services/backend-api';

export function setupTxPowerPanel(): void { ... }

/** Called by serial-panel after successful connect to sync slider with radio */
export function syncTxPowerFromRadio(): void {
  getTxPower()
    .then(watts => applyWatts(watts))
    .catch(() => {}); // no radio connected — leave slider as-is
}
```

Internal logic:
- On slider `input` event: update value display colour, call `setTxPowerConfig(watts)`
  (debounced 300 ms to avoid flooding CAT on drag)
- `applyWatts(w)`: set `slider.value`, update display text and colour class

### 7. `src/components/serial-panel.ts` — call `syncTxPowerFromRadio` on connect

After `connectSerial()` succeeds, call `syncTxPowerFromRadio()`.

### 8. `src/main.ts` — call `setupTxPowerPanel()`

Add alongside other `setup*` calls.

### 9. `src/styles.css` — TX Power panel styles

```css
/* Slider track: colour zones across full 0–100 W range */
.tx-power-slider {
  -webkit-appearance: none;
  appearance: none;
  width: 100%;
  height: 4px;
  border-radius: 2px;
  background: linear-gradient(to right,
    #22c55e 0% 25%,    /* 0–25 W green  */
    #eab308 25% 30%,   /* 25–30 W yellow */
    #ef4444 30% 100%   /* 30–100 W red  */
  );
  cursor: pointer;
  outline: none;
}
.tx-power-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 12px; height: 12px;
  border-radius: 50%;
  background: var(--text-primary);
  cursor: pointer;
}
.tx-power-slider::-moz-range-thumb {
  width: 12px; height: 12px;
  border-radius: 50%;
  background: var(--text-primary);
  border: none;
  cursor: pointer;
}

/* Value display */
.tx-power-value { font-family: 'JetBrains Mono', monospace; font-size: 11px; min-width: 36px; text-align: right; }
.tx-power-green  { color: #22c55e; }
.tx-power-yellow { color: #eab308; }
.tx-power-red    { color: #ef4444; }

/* Zone labels under slider */
.tx-power-zones { display: flex; justify-content: space-between; margin-top: 2px; }
.tx-zone-label  { font-size: 8px; }
.tx-zone-green  { color: #22c55e; }
.tx-zone-yellow { color: #eab308; flex: 1; text-align: center; }
.tx-zone-red    { color: #ef4444; }

/* PTT state row */
.ptt-row { display: flex; align-items: center; gap: var(--gap-sm); margin-top: var(--gap-xs); }
```

---

## Behaviour Summary

| Event | Action |
|-------|--------|
| App start | Slider shows saved `tx_power_watts` from config (default 10 W) |
| Radio connect | `syncTxPowerFromRadio()` reads actual radio power → updates slider + config |
| Slider drag | Value display updates live; `setTxPowerConfig` called on settle (300 ms debounce) |
| TX start | `start_tx` reads `tx_power_watts` from config (already correct value) |
| Config load (settings dialog) | Slider updated via `setSelectedTxPower(config.tx_power_watts)` |
