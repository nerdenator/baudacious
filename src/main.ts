/**
 * PSK-31 Client - Main Application Entry
 *
 * Phases 1-3: Serial/CAT, audio input, and waterfall display.
 */

import { WaterfallDisplay } from './components/waterfall';
import type { WaterfallSettings } from './components/waterfall';
import { setupRxDisplay } from './components/rx-display';
import { setupTxInput } from './components/tx-input';
import { setupTxButtons } from './components/control-panel';
import { setupWaterfallClick, setupWaterfallControls } from './components/waterfall-controls';
import { setupThemeToggle } from './components/theme-toggle';
import { setupSerialPanel } from './components/serial-panel';
import { setupAudioPanel, resetAudioPanel, setSelectedAudioDevices } from './components/audio-panel';
import { setupStatusBar } from './components/status-bar';
import { showToast } from './components/toast';
import { setupMenuEvents } from './services/event-handlers';
import { startFftBridge, listenAudioStatus } from './services/audio-bridge';
import { startRxBridge } from './services/rx-bridge';
import { startSerialBridge } from './services/serial-bridge';
import { appendRxText } from './components/rx-display';
import { loadConfiguration, saveConfiguration } from './services/backend-api';
import { setupSettingsDialog } from './components/settings-dialog';
import type { Configuration } from './types';

window.addEventListener('DOMContentLoaded', () => {
  const canvas = document.getElementById('waterfall-canvas') as HTMLCanvasElement;
  let waterfall: WaterfallDisplay | null = null;

  if (canvas) {
    waterfall = new WaterfallDisplay(canvas);
    waterfall.start();
  }

  setupTxInput();
  setupTxButtons();
  setupRxDisplay();
  setupWaterfallClick(waterfall);
  setupThemeToggle();
  setupSerialPanel();
  setupAudioPanel();
  setupMenuEvents();

  // Status bar — after serial/audio panels so they can fire setters on connect
  setupStatusBar().catch((err) => {
    console.error('Failed to set up status bar:', err);
  });

  // Wire up audio bridge: FFT events → waterfall; error status → toast + reset UI
  if (waterfall) {
    startFftBridge(waterfall);
  }
  listenAudioStatus((status) => {
    if (status.startsWith('error:')) {
      resetAudioPanel();
      showToast('Audio device lost', 'error');
    }
  });

  // Wire up serial bridge: backend-initiated disconnect → toast + reset UI
  startSerialBridge().catch((err) => {
    console.error('Failed to start serial bridge:', err);
  });

  // Wire up RX bridge: decoded text events → RX display
  startRxBridge(appendRxText).catch((err) => {
    console.error('Failed to start RX bridge:', err);
  });

  // ── Shared config state ───────────────────────────────────────────────────
  let currentConfig: Configuration | null = null;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  function scheduleConfigSave(): void {
    if (!currentConfig) return;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveConfiguration(currentConfig!).catch((err) => {
        console.warn('Failed to save config:', err);
      });
    }, 500);
  }

  // ── Waterfall controls + persistence ──────────────────────────────────────
  let applyWaterfallSettings: ((p: string, f: number, z: number) => void) | null = null;

  if (waterfall) {
    const wf = waterfall;
    applyWaterfallSettings = setupWaterfallControls(wf, (settings: WaterfallSettings) => {
      if (!currentConfig) {
        currentConfig = {
          name: 'Default',
          audio_input: null,
          audio_output: null,
          serial_port: null,
          baud_rate: 38400,
          radio_type: 'FT-991A',
          carrier_freq: 1000.0,
          waterfall_palette: settings.palette,
          waterfall_noise_floor: settings.noiseFloor,
          waterfall_zoom: settings.zoomLevel,
          tx_power_watts: 25,
        };
      } else {
        currentConfig.waterfall_palette = settings.palette;
        currentConfig.waterfall_noise_floor = settings.noiseFloor;
        currentConfig.waterfall_zoom = settings.zoomLevel;
      }
      scheduleConfigSave();
    });
  }

  // ── Settings dialog ───────────────────────────────────────────────────────
  setupSettingsDialog({
    getCurrentConfig: () => currentConfig,
    onSave: async (config) => {
      await saveConfiguration(config);
      currentConfig = config;
      applyWaterfallSettings?.(
        config.waterfall_palette,
        config.waterfall_noise_floor,
        config.waterfall_zoom,
      );
      setSelectedAudioDevices(config.audio_input, config.audio_output);
      showToast('Settings saved', 'info');
    },
  });

  // ── Load default config on startup ────────────────────────────────────────
  loadConfiguration('Default')
    .then((config) => {
      currentConfig = config;
      applyWaterfallSettings?.(
        config.waterfall_palette,
        config.waterfall_noise_floor,
        config.waterfall_zoom,
      );
    })
    .catch(() => {
      // No saved config yet — defaults already applied
    });
});
