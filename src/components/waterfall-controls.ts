/** Waterfall interaction and controls */

import { setCarrierFrequency } from '../services/backend-api';
import type { WaterfallDisplay, WaterfallSettings, ZoomLevel } from './waterfall';
import { VALID_PALETTES } from '../utils/color-map';
import type { ColorPalette } from '../utils/color-map';

/** Update the 5 frequency scale labels based on the visible Hz range */
function updateScale(startHz: number, endHz: number): void {
  const spans = document.querySelectorAll<HTMLElement>('.waterfall-scale span');
  if (spans.length !== 5) return;
  const step = (endHz - startHz) / 4;
  spans.forEach((span, i) => {
    const hz = Math.round(startHz + i * step);
    span.textContent = i === 4 ? `${hz} Hz` : `${hz}`;
  });
}

/** Wire up click-to-tune on the waterfall canvas */
export function setupWaterfallClick(waterfall: WaterfallDisplay | null): void {
  const canvas = document.getElementById('waterfall-canvas') as HTMLCanvasElement;
  const freqDisplay = document.querySelector('.waterfall-freq') as HTMLElement;
  const carrierMarker = document.querySelector('.carrier-marker') as HTMLElement;
  const statusCarrier = document.querySelector('.status-item .value.highlight') as HTMLElement;

  if (!canvas) return;

  canvas.addEventListener('click', (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;

    // Map pixel to Hz within the current visible range
    const range = waterfall ? waterfall.getVisibleRange() : { startHz: 500, endHz: 2500 };
    const freq = Math.round(range.startHz + (x / rect.width) * (range.endHz - range.startHz));

    // Update carrier frequency in the waterfall (for zoom centering)
    waterfall?.setCarrierFreq(freq);

    // Update displays
    freqDisplay.textContent = `${freq} Hz`;
    if (statusCarrier) statusCarrier.textContent = `${freq} Hz`;

    // Update carrier marker — position relative to the current visible range
    const markerX = ((freq - range.startHz) / (range.endHz - range.startHz)) * 100;
    carrierMarker.style.left = `${markerX}%`;

    // Update scale labels — visible range shifts when zoomed and carrier changes
    if (waterfall) {
      const newRange = waterfall.getVisibleRange();
      updateScale(newRange.startHz, newRange.endHz);
    }

    // Update sidebar audio carrier display
    const audioCarrierValue = document.querySelector(
      '.sidebar-section:nth-child(3) .frequency-value',
    ) as HTMLElement;
    if (audioCarrierValue) {
      audioCarrierValue.textContent = freq.toString();
    }

    // Tell the backend decoder to retune
    setCarrierFrequency(freq).catch((err) => {
      console.warn('Failed to set carrier frequency:', err);
    });
  });
}

/**
 * Wire up palette, zoom, and gain controls.
 * Returns an `applySettings` function that syncs the UI and waterfall to
 * persisted values — reusing the DOM refs already queried here.
 */
export function setupWaterfallControls(
  waterfall: WaterfallDisplay,
  onSettingsChange: (settings: WaterfallSettings) => void,
): (palette: string, noiseFloor: number, zoomLevel: number) => void {
  const paletteSelect = document.getElementById('wf-palette') as HTMLSelectElement;
  const gainSlider = document.getElementById('wf-gain') as HTMLInputElement;
  const gainDisplay = document.getElementById('wf-gain-value') as HTMLElement;
  const zoomBtns = document.querySelectorAll<HTMLButtonElement>('.wf-zoom-btn');

  function emit(): void {
    onSettingsChange(waterfall.getSettings());
    const range = waterfall.getVisibleRange();
    updateScale(range.startHz, range.endHz);
  }

  if (paletteSelect && gainSlider && gainDisplay && zoomBtns.length) {
    paletteSelect.addEventListener('change', () => {
      waterfall.setPalette(paletteSelect.value as ColorPalette);
      emit();
    });

    gainSlider.addEventListener('input', () => {
      const val = parseInt(gainSlider.value, 10);
      waterfall.setNoiseFloor(val);
      gainDisplay.textContent = `${val}`;
      emit();
    });

    zoomBtns.forEach((btn) => {
      btn.addEventListener('click', () => {
        const level = parseInt(btn.dataset['zoom'] ?? '1', 10) as ZoomLevel;
        waterfall.setZoom(level);
        zoomBtns.forEach((b) => b.classList.toggle('active', b === btn));
        emit();
      });
    });
  }

  return function applySettings(palette: string, noiseFloor: number, zoomLevel: number): void {
    const safePalette = (VALID_PALETTES.includes(palette as ColorPalette)
      ? palette
      : 'classic') as ColorPalette;
    const safeZoom = ([1, 2, 4].includes(zoomLevel) ? zoomLevel : 1) as ZoomLevel;

    waterfall.setPalette(safePalette);
    waterfall.setNoiseFloor(noiseFloor);
    waterfall.setZoom(safeZoom);

    if (paletteSelect) paletteSelect.value = safePalette;
    if (gainSlider) {
      gainSlider.value = String(noiseFloor);
      if (gainDisplay) gainDisplay.textContent = `${noiseFloor}`;
    }
    zoomBtns.forEach((btn) => {
      btn.classList.toggle('active', parseInt(btn.dataset['zoom'] ?? '1', 10) === safeZoom);
    });

    const { startHz, endHz } = waterfall.getVisibleRange();
    updateScale(startHz, endHz);
  };
}
