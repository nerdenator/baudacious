/** TX power slider + numeric input — colour-coded 0–25 W green, 25–30 W yellow, 30–100 W red */

import { getTxPower, setTxPowerConfig } from '../services/backend-api';
import { onSerialChanged } from '../services/app-state';

let _syncFromRadio: (() => void) | null = null;

/** Call after successful radio connect to sync slider with actual radio power */
export function syncTxPowerFromRadio(): void {
  _syncFromRadio?.();
}

export function setupTxPowerPanel(): void {
  const slider = document.getElementById('tx-power-slider') as HTMLInputElement | null;
  const numInput = document.getElementById('tx-power-value') as HTMLInputElement | null;
  const downBtn = document.getElementById('tx-power-down') as HTMLButtonElement | null;
  const upBtn = document.getElementById('tx-power-up') as HTMLButtonElement | null;

  if (!slider || !numInput) return;

  slider.disabled = true;
  numInput.disabled = true;

  onSerialChanged((connected) => {
    slider.disabled = !connected;
    numInput.disabled = !connected;
    if (downBtn) downBtn.disabled = !connected;
    if (upBtn) upBtn.disabled = !connected;
  });

  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  function commit(w: number): void {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      setTxPowerConfig(w).catch(err => console.error('Failed to save TX power:', err));
    }, 300);
  }

  function applyWatts(w: number): void {
    slider!.value = String(w);
    numInput!.value = String(w);
    updateColour(w);
  }

  function updateColour(w: number): void {
    numInput!.classList.remove('tx-power-green', 'tx-power-yellow', 'tx-power-red');
    if (w <= 25) {
      numInput!.classList.add('tx-power-green');
    } else if (w <= 30) {
      numInput!.classList.add('tx-power-yellow');
    } else {
      numInput!.classList.add('tx-power-red');
    }
  }

  // Slider → numeric input
  slider.addEventListener('input', () => {
    const w = parseInt(slider.value, 10);
    numInput.value = String(w);
    updateColour(w);
    commit(w);
  });

  // Numeric input → slider (on Enter or blur)
  function commitNumInput(): void {
    let w = parseInt(numInput!.value, 10);
    if (isNaN(w)) w = parseInt(slider!.value, 10);
    w = Math.max(0, Math.min(100, w));
    applyWatts(w);
    commit(w);
  }

  numInput.addEventListener('change', commitNumInput);
  numInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      commitNumInput();
      numInput.blur();
    }
  });

  downBtn?.addEventListener('click', () => {
    let w = Math.max(0, parseInt(slider.value, 10) - 1);
    applyWatts(w);
    commit(w);
  });

  upBtn?.addEventListener('click', () => {
    let w = Math.min(100, parseInt(slider.value, 10) + 1);
    applyWatts(w);
    commit(w);
  });

  _syncFromRadio = () => {
    getTxPower()
      .then(w => applyWatts(w))
      .catch(() => {}); // no radio connected — leave as-is
  };
}
