import { test, expect } from '@playwright/test';
import { mockInvoke, fireEvent, dismissStartupDialog } from './helpers';

/**
 * Phase 6 E2E tests for error handling, status bar, and the full application flow.
 *
 * Tests backend-initiated error events (serial disconnect, audio hot-plug),
 * status bar hydration on load, and a composed end-to-end smoke test.
 */

/** Minimal mocks for tests that don't need serial/audio devices */
const DISCONNECTED_STATUS = {
  get_connection_status: {
    serialConnected: false,
    serialPort: null,
    audioStreaming: false,
    audioDevice: null,
  },
  list_audio_devices: [],
  list_serial_ports: [],
};

/** Auto-connect mocks for tests that need a connected radio */
const AUTO_CONNECT_MOCKS = {
  get_connection_status: {
    serialConnected: false,
    serialPort: null,
    audioStreaming: false,
    audioDevice: null,
  },
  load_configuration: {
    name: 'Default',
    audio_input: null,
    audio_output: null,
    serial_port: '/dev/cu.usbserial-1420',
    baud_rate: 38400,
    radio_type: 'FT-991A',
    carrier_freq: 1000.0,
    waterfall_palette: 'classic',
    waterfall_noise_floor: -100,
    waterfall_zoom: 1,
    tx_power_watts: 10,
  },
  connect_serial: {
    port: '/dev/cu.usbserial-1420',
    baudRate: 38400,
    frequencyHz: 14070000,
    mode: 'DATA-USB',
    connected: true,
  },
  list_serial_ports: [
    { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)', deviceHint: null },
  ],
};

test.describe('Status Bar', () => {
  test('shows disconnected indicators on page load', async ({ page }) => {
    await mockInvoke(page, DISCONNECTED_STATUS);
    await page.goto('/');

    await expect(page.locator('#statusbar-serial .status-dot')).toHaveClass(/disconnected/);
    await expect(page.locator('#statusbar-serial .status-text')).toHaveText('CAT');
    await expect(page.locator('#statusbar-audio .status-dot')).toHaveClass(/disconnected/);
    await expect(page.locator('#statusbar-audio .status-text')).toHaveText('Audio');
  });

  test('shows connected serial indicator when seeded from backend', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: true,
        serialPort: '/dev/cu.usbserial-1420',
        audioStreaming: false,
        audioDevice: null,
      },
      list_audio_devices: [],
      list_serial_ports: [],
    });
    await page.goto('/');

    await expect(page.locator('#statusbar-serial .status-dot')).toHaveClass(/connected/);
    // Port name is truncated to 18 chars; assert partial match
    await expect(page.locator('#statusbar-serial .status-text')).toContainText('usbserial');
  });


});

test.describe('Error Toasts', () => {
  test('serial-disconnected event shows error toast with port name', async ({ page }) => {
    await mockInvoke(page, DISCONNECTED_STATUS);
    await page.goto('/');

    await fireEvent(page, 'serial-disconnected', {
      reason: 'Device disconnected',
      port: '/dev/cu.usbserial-1420',
    });

    await expect(page.locator('.toast.toast-error')).toContainText(
      'CAT disconnected: /dev/cu.usbserial-1420',
    );
  });

  test('serial-disconnected event resets serial panel to N/C', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      list_audio_devices: [],
    });
    await page.goto('/');

    // Wait for auto-connect to complete — CAT should be connected
    await expect(page.locator('#cat-status .status-dot')).toHaveClass(/connected/, { timeout: 5000 });

    // Backend fires a disconnect event
    await fireEvent(page, 'serial-disconnected', {
      reason: 'Device lost',
      port: '/dev/cu.usbserial-1420',
    });

    // Panel should reset
    await expect(page.locator('#cat-status .status-text')).toHaveText('N/C');
    await expect(page.locator('#cat-status .status-dot')).toHaveClass(/disconnected/);
    await expect(page.locator('#radio-port-name')).toHaveText('Not connected');
  });

  test('audio-status error event shows error toast', async ({ page }) => {
    await mockInvoke(page, DISCONNECTED_STATUS);
    await page.goto('/');

    await fireEvent(page, 'audio-status', { status: 'error: audio device lost' });

    await expect(page.locator('.toast.toast-error')).toContainText('Audio device lost');
  });

  test('audio-status error event resets audio panel', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false,
        serialPort: null,
        audioStreaming: false,
        audioDevice: null,
      },
      list_audio_devices: [
        { id: 'mic-1', name: 'Built-in Microphone', isInput: true, isDefault: true },
      ],
      start_audio_stream: null,
      list_serial_ports: [],
    });
    await page.goto('/');
    await dismissStartupDialog(page);

    // Start audio so the panel is in streaming state
    await page.locator('#audio-input').selectOption('mic-1');
    await expect(page.locator('#audio-in-status .status-dot')).toHaveClass(/connected/);

    // Backend fires an audio error event
    await fireEvent(page, 'audio-status', { status: 'error: audio device lost' });

    // Panel should reset
    await expect(page.locator('#audio-in-status .status-text')).toHaveText('N/C');
    await expect(page.locator('#audio-in-status .status-dot')).toHaveClass(/disconnected/);
    await expect(page.locator('#audio-input')).toHaveValue('');
  });
});

test.describe('Full Application Flow', () => {
  test('auto-connect serial → start audio → receive text → transmit → disconnect', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      disconnect_serial: null,
      list_audio_devices: [
        { id: 'mic-1', name: 'FT-991A USB Audio CODEC', isInput: true, isDefault: false },
      ],
      start_audio_stream: null,
      stop_audio_stream: null,
      start_rx: null,
      start_tx: null,
      stop_tx: null,
    });

    // When start_tx is called, fire a tx-status:complete event after a short delay
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'start_tx') {
          setTimeout(() => {
            (window as any).__dispatchTauriEvent__('tx-status', {
              status: 'complete',
              progress: 1.0,
            });
          }, 200);
        }
        return orig(cmd, args);
      };
    });

    await page.goto('/');

    // ── Step 1: Auto-connect serial ─────────────────────────────────────────
    // Frequency and mode update from the connect response
    await expect(page.locator('#freq-mhz-input')).toHaveValue('14.070', { timeout: 5000 });
    await expect(page.locator('#band-select')).toHaveValue('20m');
    await expect(page.locator('.frequency-mode')).toHaveText('DATA-USB');

    // Status bar serial indicator shows connected
    await expect(page.locator('#statusbar-serial .status-dot')).toHaveClass(/connected/);

    // ── Step 2: Start audio ─────────────────────────────────────────────────
    await page.locator('#audio-input').selectOption('mic-1');
    await expect(page.locator('#audio-in-status .status-dot')).toHaveClass(/connected/);
    await expect(page.locator('#statusbar-audio .status-dot')).toHaveClass(/connected/);

    // ── Step 3: Receive text (simulated via DOM — real event path tested in rx.spec.ts) ──
    await page.evaluate(() => {
      document.getElementById('rx-content')!.textContent = 'CQ CQ DE W1AW';
    });
    await expect(page.locator('#rx-content')).toHaveText('CQ CQ DE W1AW');

    // ── Step 4: Transmit ────────────────────────────────────────────────────
    // Add audio output device so TX path can proceed
    await page.evaluate(() => {
      const select = document.getElementById('audio-output') as HTMLSelectElement;
      const opt = document.createElement('option');
      opt.value = 'test-speaker';
      opt.textContent = 'Test Speaker';
      select.appendChild(opt);
      select.value = 'test-speaker';
    });

    await page.locator('#tx-input').fill('73 DE KD9ABC');
    await page.locator('.tx-btn-send').click();

    // UI switches to TX immediately
    await expect(page.locator('.ptt-indicator')).toHaveText('TX');

    // tx-status:complete event fires at 200ms → UI returns to RX
    await expect(page.locator('.ptt-indicator')).toHaveText('RX', { timeout: 3000 });

    // ── Step 5: Disconnect serial ───────────────────────────────────────────
    await expect(page.locator('#radio-disconnect-btn')).toBeVisible();
    await page.locator('#radio-disconnect-btn').click();

    await expect(page.locator('#cat-status .status-text')).toHaveText('N/C');
    await expect(page.locator('#radio-port-name')).toHaveText('Not connected');
  });
});

test.describe('TX PTT Regression', () => {
  /**
   * Regression: onError in control-panel.ts did not call stopTx() before clearing TX
   * state, so PTT was never released when the backend reported a TX failure.
   * Verify that a tx-status:error event causes stop_tx to be invoked.
   */
  test('tx-status error event calls stop_tx and returns UI to RX', async ({ page }) => {
    const stopTxCalls: string[] = [];

    await mockInvoke(page, {
      ...DISCONNECTED_STATUS,
      // Override to connected so the send button is enabled
      get_connection_status: {
        serialConnected: true,
        serialPort: '/dev/cu.test',
        audioStreaming: false,
        audioDevice: null,
      },
      start_tx: null,
      stop_tx: null,
    });

    // Intercept invoke to record stop_tx calls and fire an error event on start_tx
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'stop_tx') {
          (window as any).__stopTxCallCount__ = ((window as any).__stopTxCallCount__ ?? 0) + 1;
        }
        if (cmd === 'start_tx') {
          setTimeout(() => {
            (window as any).__dispatchTauriEvent__('tx-status', {
              status: 'error',
              progress: 0.0,
            });
          }, 100);
        }
        return orig(cmd, args);
      };
    });

    await page.goto('/');
    await dismissStartupDialog(page);

    // Inject an audio output option so the TX send button is not blocked
    await page.evaluate(() => {
      const select = document.getElementById('audio-output') as HTMLSelectElement;
      const opt = document.createElement('option');
      opt.value = 'test-out';
      opt.textContent = 'Test Output';
      select.appendChild(opt);
      select.value = 'test-out';
    });

    await page.locator('#tx-input').fill('TEST');
    await page.locator('.tx-btn-send').click();

    // UI must enter TX state immediately
    await expect(page.locator('.ptt-indicator')).toHaveText('TX');

    // After the error event fires, UI must return to RX
    await expect(page.locator('.ptt-indicator')).toHaveText('RX', { timeout: 3000 });

    // stop_tx must have been called (PTT released) — regression check
    const count = await page.evaluate(() => (window as any).__stopTxCallCount__ ?? 0);
    expect(count).toBeGreaterThanOrEqual(1);
  });
});
