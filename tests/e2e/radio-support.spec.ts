import { test, expect } from './fixtures';
import { mockInvoke, fireEvent } from './helpers';

/**
 * E2E tests for multi-radio support (FT-991A / FT-891):
 * radio type dropdown, baud suggestion, radioType passed to connect_serial,
 * and per-radio band selector contents.
 */

const DEFAULT_CONFIG = {
  name: 'Default',
  audio_input: null,
  audio_output: null,
  serial_port: null,
  baud_rate: 38400,
  radio_type: 'FT-991A',
  carrier_freq: 1000.0,
  waterfall_palette: 'classic',
  waterfall_noise_floor: -100,
  waterfall_zoom: 1,
  tx_power_watts: 10,
};

const BASE_MOCKS = {
  get_connection_status: {
    serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
  },
  list_configurations: ['Default'],
  load_configuration: DEFAULT_CONFIG,
  list_audio_devices: [],
  save_configuration: null,
  connect_serial: {
    port: '/dev/cu.usbserial-1420',
    baudRate: 9600,
    frequencyHz: 14070000,
    mode: 'DATA-USB',
    connected: true,
  },
  list_serial_ports: [
    { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)', deviceHint: null },
  ],
};

async function openRadioTab(page: import('@playwright/test').Page) {
  await fireEvent(page, 'menu-event', { id: 'settings' });
  await expect(page.locator('.settings-overlay')).toHaveClass(/settings-visible/);
  await page.locator('.settings-tab[data-tab="radio"]').click();
  const panel = page.locator('section.settings-panel').nth(2);
  return {
    panel,
    portSelect: panel.locator('.device-select').nth(0),
    radioSelect: panel.locator('.device-select').nth(1),
    baudSelect: panel.locator('.device-select').nth(2),
  };
}

/** Record every connect_serial invocation's args on window.__connectArgs. */
async function recordConnectArgs(page: import('@playwright/test').Page) {
  await page.addInitScript(() => {
    const w = window as any;
    w.__connectArgs = [];
    const orig = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = (cmd: string, args?: any, ...rest: any[]) => {
      if (cmd === 'connect_serial') w.__connectArgs.push(args);
      return orig(cmd, args, ...rest);
    };
  });
}

async function testConnection(page: import('@playwright/test').Page, tabs: Awaited<ReturnType<typeof openRadioTab>>) {
  await tabs.panel.locator('.refresh-btn').click();
  await tabs.portSelect.selectOption('/dev/cu.usbserial-1420');
  await page.locator('.settings-test-btn').click();
  await expect(page.locator('.settings-test-status')).toHaveClass(/success/, { timeout: 5000 });
}

test.describe('Radio support (FT-991A / FT-891)', () => {
  test('radio type dropdown lists FT-991A and FT-891', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    const { radioSelect } = await openRadioTab(page);
    await expect(radioSelect.locator('option')).toHaveText(['FT-991A', 'FT-891']);
  });

  test('selecting FT-891 suggests 9600 baud; switching back restores 38400', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    const { radioSelect, baudSelect } = await openRadioTab(page);

    await radioSelect.selectOption('FT-891');
    await expect(baudSelect).toHaveValue('9600');

    await radioSelect.selectOption('FT-991A');
    await expect(baudSelect).toHaveValue('38400');
  });

  test('baud list includes 4800 for the FT-891', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    const { baudSelect } = await openRadioTab(page);
    await expect(baudSelect.locator('option[value="4800"]')).toHaveCount(1);
  });

  test('Test Connection sends radioType FT-891 to connect_serial', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await recordConnectArgs(page);
    await page.goto('/');
    const tabs = await openRadioTab(page);

    await tabs.radioSelect.selectOption('FT-891');
    await testConnection(page, tabs);

    const args = await page.evaluate(() => (window as any).__connectArgs);
    expect(args).toHaveLength(1);
    expect(args[0]).toMatchObject({ radioType: 'FT-891', baudRate: 9600 });
  });

  test('Test Connection sends radioType FT-991A by default', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await recordConnectArgs(page);
    await page.goto('/');
    const tabs = await openRadioTab(page);

    await testConnection(page, tabs);

    const args = await page.evaluate(() => (window as any).__connectArgs);
    expect(args[0]).toMatchObject({ radioType: 'FT-991A', baudRate: 38400 });
  });

  test('FT-891 connected: band selector has no 2m or 70cm', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    const tabs = await openRadioTab(page);

    await tabs.radioSelect.selectOption('FT-891');
    await testConnection(page, tabs);

    const bands = page.locator('#band-select option');
    await expect(bands.first()).toHaveText('160m');
    await expect(page.locator('#band-select option[value="6m"]')).toHaveCount(1);
    await expect(page.locator('#band-select option[value="2m"]')).toHaveCount(0);
    await expect(page.locator('#band-select option[value="70cm"]')).toHaveCount(0);
  });

  test('FT-991A connected: band selector includes 6m, 2m and 70cm', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    const tabs = await openRadioTab(page);

    await testConnection(page, tabs);

    for (const band of ['6m', '2m', '70cm']) {
      await expect(page.locator(`#band-select option[value="${band}"]`)).toHaveCount(1);
    }
  });

  test('saved FT-891 profile auto-connects with radioType FT-891', async ({ page }) => {
    await mockInvoke(page, {
      ...BASE_MOCKS,
      load_configuration: { ...DEFAULT_CONFIG, serial_port: '/dev/cu.usbserial-1420', baud_rate: 9600, radio_type: 'FT-891' },
    });
    await recordConnectArgs(page);
    await page.goto('/');

    await expect(page.locator('#band-select option[value="2m"]')).toHaveCount(0, { timeout: 5000 });
    const args = await page.evaluate(() => (window as any).__connectArgs);
    expect(args[0]).toMatchObject({ radioType: 'FT-891', baudRate: 9600 });
  });
});
