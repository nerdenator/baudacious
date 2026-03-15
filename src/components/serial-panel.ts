/** Serial panel — manages band selector + per-band frequency input,
 *  and exposes connectFromConfig() for auto-connect / settings Test Connection.
 */

import { connectSerial, disconnectSerial, setFrequency, setMode, getRadioState } from '../services/backend-api';
import { setSerialState } from '../services/app-state';
import { syncTxPowerFromRadio } from './tx-power-panel';
import type { RadioInfo } from '../types';

type BandEntry = {
  readonly name: string;
  readonly minHz: number;
  readonly maxHz: number;
  readonly psk31Hz: number;
};

const BAND_PLAN: readonly BandEntry[] = [
  { name: '160m', minHz: 1_800_000,  maxHz: 2_000_000,  psk31Hz: 1_838_000  },
  { name: '80m',  minHz: 3_500_000,  maxHz: 4_000_000,  psk31Hz: 3_580_000  },
  { name: '40m',  minHz: 7_000_000,  maxHz: 7_300_000,  psk31Hz: 7_035_000  },
  { name: '30m',  minHz: 10_100_000, maxHz: 10_150_000, psk31Hz: 10_142_000 },
  { name: '20m',  minHz: 14_000_000, maxHz: 14_350_000, psk31Hz: 14_070_000 },
  { name: '17m',  minHz: 18_068_000, maxHz: 18_168_000, psk31Hz: 18_100_000 },
  { name: '15m',  minHz: 21_000_000, maxHz: 21_450_000, psk31Hz: 21_080_000 },
  { name: '12m',  minHz: 24_890_000, maxHz: 24_990_000, psk31Hz: 24_920_000 },
  { name: '10m',  minHz: 28_000_000, maxHz: 29_700_000, psk31Hz: 28_120_000 },
];

/** Return the correct DATA mode for a frequency.
 *  Mirrors data_mode_for_frequency() in domain/frequency.rs:
 *  60m is always DATA-USB; below 10 MHz is DATA-LSB; 10 MHz+ is DATA-USB. */
function dataModeForHz(hz: number): string {
  if (hz >= 5_332_000 && hz <= 5_405_000) return 'DATA-USB'; // 60m exception
  return hz < 10_000_000 ? 'DATA-LSB' : 'DATA-USB';
}

function detectBand(hz: number): BandEntry | null {
  return BAND_PLAN.find(b => hz >= b.minHz && hz <= b.maxHz) ?? null;
}

function applyBandToInput(band: BandEntry, input: HTMLInputElement, hint: HTMLElement | null): void {
  const minMhz = band.minHz / 1e6;
  const maxMhz = band.maxHz / 1e6;
  input.min = minMhz.toFixed(3);
  input.max = maxMhz.toFixed(3);
  input.value = (band.psk31Hz / 1e6).toFixed(3);
  if (hint) hint.textContent = `(${minMhz.toFixed(3)}–${maxMhz.toFixed(3)})`;
}

let _resetUi: (() => void) | null = null;

/** Reset the serial panel to disconnected state (e.g. on backend-initiated disconnect) */
export function resetSerialPanel(): void {
  _resetUi?.();
}

/**
 * Connect to a serial port using the given parameters, then run all post-connect
 * setup (band detect, mode correction, poll start, sidebar update, TX power sync).
 *
 * Used by auto-connect on startup and Settings → Test Connection.
 */
export async function connectFromConfig(port: string, baudRate: number): Promise<void> {
  const info = await connectSerial(port, baudRate);
  handleConnectSuccess(info);
}

/**
 * Post-connect logic — updates sidebar, starts polling, syncs TX power.
 * Exported so main.ts can call it on reload when the backend is already connected.
 */
export function handleConnectSuccess(info: RadioInfo): void {
  const portNameEl = document.getElementById('radio-port-name');
  const disconnectBtn = document.getElementById('radio-disconnect-btn') as HTMLButtonElement | null;
  const bandSelect = document.getElementById('band-select') as HTMLSelectElement;
  const freqInput = document.getElementById('freq-mhz-input') as HTMLInputElement;
  const rangeHint = document.getElementById('freq-range-hint') as HTMLElement;
  const freqMode = document.getElementById('frequency-mode') as HTMLElement;
  const catDot = document.querySelector('#cat-status .status-dot') as HTMLElement;
  const catText = document.querySelector('#cat-status .status-text') as HTMLElement;

  // Update sidebar status row
  if (portNameEl) portNameEl.textContent = info.port;
  if (disconnectBtn) disconnectBtn.style.display = '';

  // Detect band from reported frequency and update controls
  const band = detectBand(info.frequencyHz);
  _activeBand = band;
  bandSelect.disabled = false;
  freqInput.disabled = false;
  if (band) {
    bandSelect.value = band.name;
    applyBandToInput(band, freqInput, rangeHint);
    // Show actual current freq (may differ from the calling freq)
    freqInput.value = (info.frequencyHz / 1e6).toFixed(3);
  } else {
    bandSelect.value = '';
    freqInput.min = '1.8';
    freqInput.max = '30';
    freqInput.value = (info.frequencyHz / 1e6).toFixed(3);
    if (rangeHint) rangeHint.textContent = '';
  }
  if (freqMode) freqMode.textContent = info.mode;

  // Update CAT status indicator
  if (catDot) {
    catDot.classList.remove('disconnected');
    catDot.classList.add('connected');
  }
  if (catText) {
    catText.classList.remove('disconnected');
    catText.classList.add('connected');
    catText.textContent = 'OK';
  }

  // Mark connected in app-state
  setSerialState(true, info.port);

  // Sync TX power slider with actual radio power
  syncTxPowerFromRadio();

  // Start polling radio state every 2s to track knob/VFO changes on the rig
  _connected = true;
  if (_pollInterval) clearInterval(_pollInterval);
  _pollInterval = window.setInterval(_syncRadioState, 2_000);
}

// Module-level state shared between handleConnectSuccess and the event handlers
// set up in setupSerialPanel().
let _connected = false;
let _pollInterval: number | null = null;
let _activeBand: BandEntry | null = null;
let _lastUserActionAt = 0;
const USER_ACTION_SUPPRESS_MS = 4_000;

function _syncRadioState(): void {
  const bandSelect = document.getElementById('band-select') as HTMLSelectElement;
  const freqInput = document.getElementById('freq-mhz-input') as HTMLInputElement;
  const rangeHint = document.getElementById('freq-range-hint') as HTMLElement;
  const freqMode = document.getElementById('frequency-mode') as HTMLElement;

  if (Date.now() - _lastUserActionAt < USER_ACTION_SUPPRESS_MS) return;
  if (document.activeElement === freqInput) return;
  getRadioState().then(status => {
    if (!_connected) return;
    if (Date.now() - _lastUserActionAt < USER_ACTION_SUPPRESS_MS) return;
    if (document.activeElement === freqInput) return;
    const hz = status.frequencyHz;
    const band = detectBand(hz);
    _activeBand = band;
    if (band) {
      bandSelect.value = band.name;
      freqInput.min = (band.minHz / 1e6).toFixed(3);
      freqInput.max = (band.maxHz / 1e6).toFixed(3);
      if (rangeHint) rangeHint.textContent = `(${(band.minHz / 1e6).toFixed(3)}–${(band.maxHz / 1e6).toFixed(3)})`;
    } else {
      bandSelect.value = '';
    }
    freqInput.value = (hz / 1e6).toFixed(3);
    if (freqMode) freqMode.textContent = status.mode;
  }).catch(() => { /* radio may be briefly busy */ });
}

export function setupSerialPanel(onOpenSettings: (tab: 'radio' | 'general') => void): void {
  const configureBtn = document.getElementById('radio-configure-btn') as HTMLButtonElement;
  const disconnectBtn = document.getElementById('radio-disconnect-btn') as HTMLButtonElement;
  const portNameEl = document.getElementById('radio-port-name');
  const bandSelect = document.getElementById('band-select') as HTMLSelectElement;
  const freqInput = document.getElementById('freq-mhz-input') as HTMLInputElement;
  const rangeHint = document.getElementById('freq-range-hint') as HTMLElement;
  const freqMode = document.getElementById('frequency-mode') as HTMLElement;

  // Populate band dropdown from BAND_PLAN
  for (const band of BAND_PLAN) {
    const opt = document.createElement('option');
    opt.value = band.name;
    opt.textContent = band.name;
    bandSelect.appendChild(opt);
  }

  // Configure button → open settings on Radio tab
  configureBtn?.addEventListener('click', () => onOpenSettings('radio'));

  // Disconnect button → call disconnectSerial then reset UI
  disconnectBtn?.addEventListener('click', async () => {
    try {
      await disconnectSerial();
    } catch (err) {
      console.error('Disconnect failed:', err);
    }
    resetUi();
  });

  // Band select change: jump to PSK-31 calling freq and set correct DATA mode
  bandSelect.addEventListener('change', () => {
    if (!_connected) return;
    _lastUserActionAt = Date.now();
    const band = BAND_PLAN.find(b => b.name === bandSelect.value) ?? null;
    if (!band) return;
    _activeBand = band;
    applyBandToInput(band, freqInput, rangeHint);
    const mode = dataModeForHz(band.psk31Hz);
    if (freqMode) freqMode.textContent = mode;
    setFrequency(band.psk31Hz).catch(err => console.error('set_frequency failed:', err));
    setMode(mode).catch(err => console.error('set_mode failed:', err));
  });

  // Freq input commit: clamp to band range + send on blur or Enter
  function commitFreq(): void {
    if (!_connected) return;
    _lastUserActionAt = Date.now();
    let mhz = parseFloat(freqInput.value);
    if (isNaN(mhz)) return;
    if (_activeBand) {
      const minMhz = _activeBand.minHz / 1e6;
      const maxMhz = _activeBand.maxHz / 1e6;
      if (mhz < minMhz || mhz > maxMhz) {
        if (rangeHint) {
          rangeHint.textContent = `Change band to enter a frequency outside ${_activeBand.name} (${minMhz.toFixed(3)}–${maxMhz.toFixed(3)} MHz)`;
          rangeHint.classList.add('range-hint-error');
          window.setTimeout(() => {
            rangeHint.textContent = `(${minMhz.toFixed(3)}–${maxMhz.toFixed(3)})`;
            rangeHint.classList.remove('range-hint-error');
          }, 4000);
        }
        // Don't send any CAT commands — restore input to band min/max edge and stop.
        freqInput.value = (Math.max(minMhz, Math.min(maxMhz, mhz))).toFixed(3);
        return;
      }
    }
    const hz = Math.round(mhz * 1e6);
    const mode = dataModeForHz(hz);
    if (freqMode) freqMode.textContent = mode;
    setFrequency(hz).catch(err => console.error('set_frequency failed:', err));
    setMode(mode).catch(err => console.error('set_mode failed:', err));
  }

  freqInput.addEventListener('blur', commitFreq);
  freqInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); commitFreq(); freqInput.blur(); }
  });

  function resetUi(): void {
    _connected = false;
    _activeBand = null;
    setSerialState(false, null);
    if (_pollInterval) {
      clearInterval(_pollInterval);
      _pollInterval = null;
    }

    // Sidebar status row
    if (portNameEl) portNameEl.textContent = 'Not connected';
    if (disconnectBtn) disconnectBtn.style.display = 'none';

    // Reset frequency controls to disabled/blank state
    bandSelect.value = '';
    bandSelect.disabled = true;
    freqInput.value = '';
    freqInput.disabled = true;
    if (rangeHint) rangeHint.textContent = '';
    if (freqMode) freqMode.textContent = '—';

    const catDot = document.querySelector('#cat-status .status-dot') as HTMLElement;
    const catText = document.querySelector('#cat-status .status-text') as HTMLElement;
    if (catDot) {
      catDot.classList.remove('connected');
      catDot.classList.add('disconnected');
    }
    if (catText) {
      catText.classList.remove('connected');
      catText.classList.add('disconnected');
      catText.textContent = 'N/C';
    }
  }

  _resetUi = resetUi;
}
