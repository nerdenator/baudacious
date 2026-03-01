/** Serial panel — populates port dropdown, wires connect/disconnect button,
 *  and manages the band selector + per-band frequency input.
 */

import { listSerialPorts, connectSerial, disconnectSerial, setFrequency } from '../services/backend-api';
import { setSerialState } from '../services/app-state';

/** Default baud rate for FT-991A */
const DEFAULT_BAUD_RATE = 38400;

/** How long the connect button stays green after a successful connection (ms) */
const SUCCESS_FLASH_MS = 10_000;

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

let _resetUi: (() => void) | null = null;

/** Reset the serial panel to disconnected state (e.g. on backend-initiated disconnect) */
export function resetSerialPanel(): void {
  _resetUi?.();
}

export function setupSerialPanel(): void {
  const dropdown = document.getElementById('serial-port') as HTMLSelectElement;
  const connectBtn = document.getElementById('serial-connect-btn') as HTMLButtonElement;
  const bandSelect = document.getElementById('band-select') as HTMLSelectElement;
  const freqInput = document.getElementById('freq-mhz-input') as HTMLInputElement;
  const rangeHint = document.getElementById('freq-range-hint') as HTMLElement;
  const freqMode = document.getElementById('frequency-mode') as HTMLElement;
  const catDot = document.querySelector('#cat-status .status-dot') as HTMLElement;
  const catText = document.querySelector('#cat-status .status-text') as HTMLElement;

  if (!dropdown || !connectBtn) return;

  let connected = false;
  let flashTimeout: number | null = null;
  let connectionId = 0;
  let activeBand: BandEntry | null = null;

  // Populate band dropdown from BAND_PLAN
  for (const band of BAND_PLAN) {
    const opt = document.createElement('option');
    opt.value = band.name;
    opt.textContent = band.name;
    bandSelect.appendChild(opt);
  }

  // Populate serial dropdown from backend on load
  populateDropdown(dropdown);

  // Band select change: jump to PSK-31 calling freq for that band
  bandSelect.addEventListener('change', () => {
    if (!connected) return;
    const band = BAND_PLAN.find(b => b.name === bandSelect.value) ?? null;
    if (!band) return;
    activeBand = band;
    applyBandToInput(band, freqInput, rangeHint);
    setFrequency(band.psk31Hz).catch(err => console.error('set_frequency failed:', err));
  });

  // Freq input commit: clamp to band range + send on blur or Enter
  function commitFreq(): void {
    if (!connected) return;
    let mhz = parseFloat(freqInput.value);
    if (isNaN(mhz)) return;
    if (activeBand) {
      mhz = Math.max(activeBand.minHz / 1e6, Math.min(activeBand.maxHz / 1e6, mhz));
      freqInput.value = mhz.toFixed(3);
    }
    setFrequency(Math.round(mhz * 1e6)).catch(err => console.error('set_frequency failed:', err));
  }

  freqInput.addEventListener('blur', commitFreq);
  freqInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); commitFreq(); freqInput.blur(); }
  });

  connectBtn.addEventListener('click', async () => {
    if (connected) {
      try {
        await disconnectSerial();
      } catch (err) {
        console.error('Disconnect failed:', err);
      }
      resetUi();
      return;
    }

    const port = dropdown.value;
    if (!port) return;

    connectionId++;
    const thisConnection = connectionId;

    connectBtn.disabled = true;
    connectBtn.textContent = 'Connecting...';

    try {
      const info = await connectSerial(port, DEFAULT_BAUD_RATE);

      if (thisConnection !== connectionId) return;

      connected = true;
      setSerialState(true, port);

      // Detect band from reported frequency and update controls
      const band = detectBand(info.frequencyHz);
      activeBand = band;
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

      // Flash button green for 10s
      connectBtn.disabled = false;
      connectBtn.textContent = 'Connected';
      connectBtn.classList.add('connected');
      dropdown.disabled = true;

      if (flashTimeout) clearTimeout(flashTimeout);
      flashTimeout = window.setTimeout(() => {
        if (thisConnection === connectionId) {
          connectBtn.classList.remove('connected');
          connectBtn.textContent = 'Disconnect';
        }
        flashTimeout = null;
      }, SUCCESS_FLASH_MS);
    } catch (err) {
      if (thisConnection !== connectionId) return;

      console.error('Connect failed:', err);
      connectBtn.disabled = false;
      connectBtn.textContent = 'Connect';

      if (catDot) {
        catDot.classList.remove('connected');
        catDot.classList.add('disconnected');
      }
      if (catText) {
        catText.classList.remove('connected');
        catText.classList.add('disconnected');
        catText.textContent = 'Error';
      }
    }
  });

  function resetUi(): void {
    connected = false;
    activeBand = null;
    setSerialState(false, null);
    connectionId++;
    if (flashTimeout) {
      clearTimeout(flashTimeout);
      flashTimeout = null;
    }
    connectBtn.disabled = false;
    connectBtn.textContent = 'Connect';
    connectBtn.classList.remove('connected');
    dropdown.disabled = false;

    // Reset frequency controls to disabled/blank state
    bandSelect.value = '';
    bandSelect.disabled = true;
    freqInput.value = '';
    freqInput.disabled = true;
    if (rangeHint) rangeHint.textContent = '';
    if (freqMode) freqMode.textContent = '—';

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

/** Fetch serial ports from backend and populate the dropdown */
async function populateDropdown(dropdown: HTMLSelectElement): Promise<void> {
  try {
    const ports = await listSerialPorts();
    while (dropdown.options.length > 1) {
      dropdown.remove(1);
    }
    for (const port of ports) {
      const option = document.createElement('option');
      option.value = port.name;
      option.textContent = `${port.name} (${port.portType})`;
      dropdown.appendChild(option);
    }
  } catch (err) {
    console.error('Failed to list serial ports:', err);
  }
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
