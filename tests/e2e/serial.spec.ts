import { test, expect } from './fixtures';
import { mockInvoke, fireEvent, dismissStartupDialog } from './helpers';

/**
 * E2E tests for serial/CAT communication.
 *
 * The serial port dropdown and Connect button have been removed from the sidebar.
 * Connection is now initiated via auto-connect on startup (using saved config) or
 * via Settings → Radio tab → Test Connection.
 *
 * Auto-connect path: mock load_configuration to return a config with serial_port set,
 * and mock connect_serial to return the expected RadioInfo.
 */

const CONNECTED_RADIO_INFO = {
  port: '/dev/cu.usbserial-1420',
  baudRate: 38400,
  frequencyHz: 14070000,
  mode: 'DATA-USB',
  connected: true,
};

/** Mocks for auto-connect success on 20m */
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
  connect_serial: CONNECTED_RADIO_INFO,
  list_serial_ports: [
    { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)', deviceHint: 'Yaesu FT-991A / CP210x' },
  ],
};

test.describe('Serial Panel', () => {
  test('auto-connect updates frequency display on success', async ({ page }) => {
    await mockInvoke(page, AUTO_CONNECT_MOCKS);
    await page.goto('/');

    // Auto-connect fires on startup — frequency and band should update
    await expect(page.locator('#freq-mhz-input')).toHaveValue('14.070', { timeout: 5000 });
    await expect(page.locator('#band-select')).toHaveValue('20m');

    // Mode should update
    const freqMode = page.locator('.frequency-mode');
    await expect(freqMode).toHaveText('DATA-USB');
  });

  test('auto-connect shows port name in sidebar', async ({ page }) => {
    await mockInvoke(page, AUTO_CONNECT_MOCKS);
    await page.goto('/');

    // Sidebar should show the connected port name (not "Not connected")
    await expect(page.locator('#radio-port-name')).toContainText('usbserial', { timeout: 5000 });
  });

  test('auto-connect shows Disconnect button', async ({ page }) => {
    await mockInvoke(page, AUTO_CONNECT_MOCKS);
    await page.goto('/');

    // Disconnect button should be visible after successful connect
    await expect(page.locator('#radio-disconnect-btn')).toBeVisible({ timeout: 5000 });
  });

  test('CAT status indicator shows connected after auto-connect', async ({ page }) => {
    await mockInvoke(page, AUTO_CONNECT_MOCKS);
    await page.goto('/');

    // CAT status dot should be connected
    await expect(page.locator('#cat-status .status-dot')).toHaveClass(/connected/, { timeout: 5000 });

    // CAT status text should say OK
    await expect(page.locator('#cat-status .status-text')).toHaveText('OK');
  });

  test('disconnect button resets UI', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      disconnect_serial: null,
    });

    await page.goto('/');

    // Wait for auto-connect to complete
    await expect(page.locator('#radio-disconnect-btn')).toBeVisible({ timeout: 5000 });

    // Click disconnect
    await page.locator('#radio-disconnect-btn').click();

    // CAT status should be disconnected
    const catText = page.locator('#cat-status .status-text');
    await expect(catText).toHaveText('N/C');

    // Disconnect button should be hidden
    await expect(page.locator('#radio-disconnect-btn')).not.toBeVisible();

    // Port name should revert
    await expect(page.locator('#radio-port-name')).toHaveText('Not connected');
  });

  // ---------------------------------------------------------------------------
  // Frequency input + band selector
  // ---------------------------------------------------------------------------

  test('band change sends PSK-31 calling frequency to radio', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      set_frequency: null,
      set_mode: null,
    });

    // Spy on set_frequency calls
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__setFrequencyCalls__ = [];
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'set_frequency') (window as any).__setFrequencyCalls__.push(args.freqHz);
        return orig(cmd, args);
      };
    });

    await page.goto('/');
    await expect(page.locator('#band-select')).toHaveValue('20m', { timeout: 5000 });

    await page.locator('#band-select').selectOption('40m');

    const calls = await page.evaluate(() => (window as any).__setFrequencyCalls__);
    expect(calls).toContain(7035000); // 40m PSK-31 calling freq
    await expect(page.locator('#freq-mhz-input')).toHaveValue('7.035');
  });

  test('out-of-band frequency entry shows error and does not call set_frequency', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      connect_serial: {
        port: '/dev/cu.usbserial-1420',
        baudRate: 38400,
        frequencyHz: 3580000,
        mode: 'DATA-LSB',
        connected: true,
      },
      set_frequency: null,
      set_mode: null,
      get_radio_state: {
        frequencyHz: 3580000, mode: 'DATA-LSB',
        isTransmitting: false, ritOffsetHz: 0, ritEnabled: false, split: false,
      },
    });

    // Spy: record set_frequency calls made AFTER connect
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__setFrequencyCalls__ = [];
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'set_frequency') (window as any).__setFrequencyCalls__.push(args.freqHz);
        return orig(cmd, args);
      };
    });

    await page.goto('/');
    await expect(page.locator('#band-select')).toHaveValue('80m', { timeout: 5000 });

    // Clear spy calls from connect, then type an out-of-band frequency
    await page.evaluate(() => { (window as any).__setFrequencyCalls__ = []; });
    await page.locator('#freq-mhz-input').fill('14.074');
    await page.locator('#freq-mhz-input').press('Enter');

    // Error hint should appear
    const hint = page.locator('#freq-range-hint');
    await expect(hint).toContainText('Change band');
    await expect(hint).toContainText('80m');

    // set_frequency must NOT have been called with the out-of-band value
    const calls = await page.evaluate(() => (window as any).__setFrequencyCalls__);
    expect(calls).not.toContain(14074000);
  });

  test('out-of-band frequency clamps input to band edge', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      connect_serial: {
        port: '/dev/cu.usbserial-1420',
        baudRate: 38400,
        frequencyHz: 3580000,
        mode: 'DATA-LSB',
        connected: true,
      },
      set_frequency: null,
      set_mode: null,
      get_radio_state: {
        frequencyHz: 3580000, mode: 'DATA-LSB',
        isTransmitting: false, ritOffsetHz: 0, ritEnabled: false, split: false,
      },
    });

    await page.goto('/');
    await expect(page.locator('#band-select')).toHaveValue('80m', { timeout: 5000 });

    await page.locator('#freq-mhz-input').fill('14.074');
    await page.locator('#freq-mhz-input').press('Enter');

    // Input should be clamped to 80m upper edge (4.000 MHz)
    const value = parseFloat(await page.locator('#freq-mhz-input').inputValue());
    expect(value).toBeLessThanOrEqual(4.000);
    expect(value).toBeGreaterThanOrEqual(3.500);
  });

  test('get_radio_state poll updates band and frequency display', async ({ page }) => {
    await mockInvoke(page, {
      ...AUTO_CONNECT_MOCKS,
      set_frequency: null,
      set_mode: null,
      get_radio_state: {
        frequencyHz: 7035000, mode: 'DATA-LSB',
        isTransmitting: false, ritOffsetHz: 0, ritEnabled: false, split: false,
      },
    });

    await page.goto('/');
    await expect(page.locator('#band-select')).toHaveValue('20m', { timeout: 5000 });

    // Wait for the 2s poll to fire and update the UI
    await expect(page.locator('#band-select')).toHaveValue('40m', { timeout: 5000 });
    await expect(page.locator('#freq-mhz-input')).toHaveValue('7.035');
    await expect(page.locator('.frequency-mode')).toHaveText('DATA-LSB');
  });

  test('auto-connect failure shows startup recovery dialog', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
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
      list_serial_ports: [],
    });

    // Override connect_serial to reject
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'connect_serial') {
          return Promise.reject('Failed to open /dev/cu.usbserial-1420: No such file');
        }
        return orig(cmd, args);
      };
    });

    await page.goto('/');

    // Startup recovery dialog should appear
    await expect(page.locator('.startup-overlay')).toBeVisible({ timeout: 5000 });
    await expect(page.locator('.startup-title')).toHaveText('Radio Not Connected');
    await expect(page.locator('.startup-body')).toContainText('No such file');
  });

  test('sidebar shows Configure button when disconnected', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      list_serial_ports: [],
    });

    await page.goto('/');
    await dismissStartupDialog(page);

    // Configure button should be visible
    await expect(page.locator('#radio-configure-btn')).toBeVisible();
    // Port name should say Not connected
    await expect(page.locator('#radio-port-name')).toHaveText('Not connected');
    // Disconnect button hidden
    await expect(page.locator('#radio-disconnect-btn')).not.toBeVisible();
  });
});
