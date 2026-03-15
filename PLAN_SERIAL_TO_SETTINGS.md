# Plan: Move Serial Configuration to Settings Dialog

## Goal

Remove the serial port dropdown and Connect button from the sidebar. Move all
radio connection configuration into the Settings dialog Radio tab. Add
auto-connect on startup with a recovery dialog on failure.

---

## New User Flow

### Startup
1. Load the last-used (Default) config profile
2. If `config.serial_port` is set → attempt `connectSerial(port, baud_rate)` silently
3. **Success** → normal operation, sidebar shows connection status
4. **Failure or no port saved** → show **Startup Recovery Dialog** with 3 choices:
   - **Select a different profile** → opens Settings → General tab (profile switcher)
   - **Reconfigure this profile** → opens Settings → Radio tab
   - **Exit** → closes the application

### Connecting / Reconfiguring
- All serial port configuration lives in **Settings → Radio tab**
- Radio tab gains: serial port dropdown + refresh button, Test Connection button with inline status
- "Test Connection" calls `connectSerial` and shows result inline (no toast)
- On success the app is live-connected; Save & Apply persists the port to config

### Sidebar (Radio Control section)
Simplified to status display only:
- Shows connected port name when connected, "Not connected" otherwise
- **Configure Radio** button → opens Settings → Radio tab
- **Disconnect** link/button → shown only when connected

---

## Tests

### Rust unit tests
- None needed — no new Tauri commands; existing `connectSerial` / `disconnectSerial` are reused

### Playwright E2E (`tests/e2e/startup.spec.ts`)
- **Auto-connect success**: mock `load_configuration` returns config with `serial_port` set,
  mock `connect_serial` succeeds → no recovery dialog shown, sidebar shows port name
- **Auto-connect failure**: mock `connect_serial` rejects → recovery dialog appears with
  all 3 options
- **Recovery → Reconfigure**: clicking "Reconfigure this profile" opens settings on Radio tab
- **Recovery → Select profile**: clicking "Select a different profile" opens settings on General tab
- **Recovery → Exit**: clicking "Exit" calls `process.exit` (or Tauri `exit` command)

### Playwright E2E (`tests/e2e/settings-radio.spec.ts`)
- Settings Radio tab shows serial port dropdown, refresh button, baud rate, test connection button
- Clicking refresh re-enumerates ports (calls `list_serial_ports`)
- "Test Connection" success: shows "Connected ✓" inline
- "Test Connection" failure: shows error message inline
- Save & Apply persists `serial_port` field in config

---

## Files to Change

### 1. `index.html` — Simplify Radio Control section

**Remove:** `#serial-port` select, `#serial-connect-btn` button

**Replace with:**
```html
<!-- Radio Control -->
<div class="sidebar-section">
  <div class="section-label">Radio Control</div>
  <div class="radio-status-row">
    <span id="radio-port-name" class="radio-port-name">Not connected</span>
    <button id="radio-configure-btn" class="radio-configure-btn">Configure</button>
  </div>
  <button id="radio-disconnect-btn" class="radio-disconnect-btn" style="display:none">
    Disconnect
  </button>
</div>
```

### 2. `src/components/serial-panel.ts` — Simplify to operational logic only

**Remove:** port enumeration, baud rate read, connect button handler, `populateDropdown()`

**Keep:** band/frequency controls, polling, `resetSerialPanel()`, `setSerialState()` calls,
`syncTxPowerFromRadio()` call on connect

**New responsibilities:**
- `setupSerialPanel(onOpenSettings: (tab: 'radio' | 'general') => void)` — takes a callback
  to open the settings dialog (wires up Configure button and Disconnect button)
- `connectFromConfig(port: string, baudRate: number): Promise<RadioInfo>` — exported helper
  that calls `connectSerial`, runs post-connect setup (band detect, mode correct, poll start),
  updates sidebar status row
- `handleConnectSuccess(info: RadioInfo): void` — extracted from current connect handler,
  shared by auto-connect and settings test-connection paths

**Sidebar status row update:**
- On connect: `#radio-port-name` ← port name, `#radio-disconnect-btn` visible
- On disconnect: `#radio-port-name` ← "Not connected", `#radio-disconnect-btn` hidden

### 3. `src/components/settings-dialog.ts` — Enhance Radio tab

**Add to Radio tab (before Radio Type):**
```typescript
// Serial Port row with refresh button
const portHeader = el('div', 'settings-device-header');
portHeader.appendChild(label('Serial Port'));
const portRefreshBtn = btn('refresh-btn', '↻');
portHeader.appendChild(portRefreshBtn);

const portSelect = select('device-select');
portSelect.appendChild(placeholder('Select port...'));

// Test Connection button + inline status
const testBtn = btn('settings-test-btn', 'Test Connection');
const testStatus = el('span', 'settings-test-status');
```

**Port refresh:** calls `listSerialPorts()`, repopulates `portSelect`

**Test Connection handler:**
1. Disable testBtn, set testStatus to "Connecting…"
2. Call `connectFromConfig(portSelect.value, parseInt(baudSelect.value))` (imported from serial-panel)
3. Success: testStatus = "Connected ✓" (green), button → "Disconnect"
4. Failure: testStatus = error message (red)

**prefillFromConfig:** sets `portSelect.value = config.serial_port ?? ''`

**Save & Apply:** includes `serial_port: portSelect.value || null` in the saved config

### 4. `src/components/startup-dialog.ts` — New file

Thin modal with 3 buttons. Shown by `main.ts` when auto-connect fails.

```typescript
export interface StartupRecoveryOptions {
  onSelectProfile: () => void;   // opens settings General tab
  onReconfigure: () => void;     // opens settings Radio tab
  onExit: () => void;
}

export function showStartupRecoveryDialog(
  profileName: string,
  error: string,
  options: StartupRecoveryOptions,
): void { ... }

export function hideStartupRecoveryDialog(): void { ... }
```

**UI:**
- Title: "Radio Not Connected"
- Body: `Could not connect to "${profileName}": <error>`
- Three buttons:
  - "Select a different profile" → `options.onSelectProfile()`
  - "Reconfigure this profile" → `options.onReconfigure()`
  - "Exit" → `options.onExit()`

### 5. `src/main.ts` — Auto-connect on startup

After loading the default config:
```typescript
const config = await loadConfiguration('Default').catch(() => null) ?? makeDefaultConfig();
deps.applyConfig(config);

if (config.serial_port) {
  try {
    await connectFromConfig(config.serial_port, config.baud_rate);
  } catch (err) {
    showStartupRecoveryDialog(config.name, String(err), {
      onSelectProfile: () => openSettingsDialog('general'),
      onReconfigure:   () => openSettingsDialog('radio'),
      onExit:          () => invoke('exit_app'),
    });
  }
} else {
  // No port configured — show recovery immediately
  showStartupRecoveryDialog(config.name, 'No serial port configured', {
    onSelectProfile: () => openSettingsDialog('general'),
    onReconfigure:   () => openSettingsDialog('radio'),
    onExit:          () => invoke('exit_app'),
  });
}
```

### 6. `src-tauri/src/commands/` — Add `exit_app` command

```rust
#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}
```

Register in `lib.rs` invoke handler.

### 7. `src/styles.css` — New styles

```css
/* Simplified sidebar radio status */
.radio-status-row { display: flex; align-items: center; justify-content: space-between; }
.radio-port-name  { font-family: 'JetBrains Mono', monospace; font-size: 10px; color: var(--text-dim); }
.radio-configure-btn  { /* same style as serial-connect-btn but smaller */ }
.radio-disconnect-btn { /* danger-tinted small text button */ }

/* Settings dialog test connection */
.settings-test-btn    { /* same family as save btn but secondary */ }
.settings-test-status { font-size: 10px; margin-left: var(--gap-sm); }
.settings-test-status.success { color: #22c55e; }
.settings-test-status.error   { color: #ef4444; }

/* Startup recovery dialog */
.startup-overlay  { /* same overlay pattern as settings */ }
.startup-dialog   { /* centered card, narrower than settings */ }
.startup-recovery-btn { /* full-width option buttons */ }
```

---

## Architecture Notes

- `connectFromConfig` is the single connection entry point — used by auto-connect
  (main.ts startup), settings Test Connection, and startup recovery post-reconfigure
- `handleConnectSuccess` updates sidebar status row, starts polling, calls
  `syncTxPowerFromRadio()` — extracted so both paths share identical post-connect logic
- Settings dialog hides the startup recovery dialog on open (in case user clicked
  "Reconfigure" from recovery dialog — close recovery, show settings)
- `openSettingsDialog` (existing export from settings-dialog.ts) now accepts
  `'radio'` as a valid tab argument to open directly to Radio tab

---

## Deferred / Out of Scope

- Baud rate auto-detection (try all rates)
- Multiple simultaneous radio profiles
- Radio-less mode (intentionally showing recovery dialog is correct UX here)
