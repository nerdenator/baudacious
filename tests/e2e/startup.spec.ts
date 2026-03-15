import { test, expect } from '@playwright/test';
import { mockInvoke, fireEvent } from './helpers';

/**
 * E2E tests for startup auto-connect and recovery dialog.
 *
 * Tests the new startup flow: load config → auto-connect → success/failure handling.
 */

const BASE_CONFIG = {
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
};

test.describe('Startup Auto-Connect', () => {
  test('auto-connect success: no recovery dialog shown, sidebar shows port name', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      load_configuration: BASE_CONFIG,
      connect_serial: {
        port: '/dev/cu.usbserial-1420',
        baudRate: 38400,
        frequencyHz: 14070000,
        mode: 'DATA-USB',
        connected: true,
      },
      list_serial_ports: [],
    });

    await page.goto('/');

    // Recovery dialog must NOT appear
    await expect(page.locator('.startup-overlay')).not.toBeVisible({ timeout: 3000 });

    // Sidebar port name should show the connected port
    await expect(page.locator('#radio-port-name')).toContainText('usbserial', { timeout: 5000 });
  });

  test('auto-connect failure: recovery dialog appears with all 4 options', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      load_configuration: BASE_CONFIG,
      list_serial_ports: [],
    });

    // Override connect_serial to reject
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'connect_serial') {
          return Promise.reject('Port not found');
        }
        return orig(cmd, args);
      };
    });

    await page.goto('/');

    // Recovery dialog must appear
    await expect(page.locator('.startup-overlay')).toBeVisible({ timeout: 5000 });
    await expect(page.locator('.startup-title')).toHaveText('Radio Not Connected');
    await expect(page.locator('.startup-body')).toContainText('Port not found');

    // All 4 buttons should be present: Retry, Select profile, Reconfigure, Exit
    const buttons = page.locator('.startup-recovery-btn');
    await expect(buttons).toHaveCount(4);
    await expect(buttons.nth(0)).toContainText('Retry connection');
    await expect(buttons.nth(1)).toContainText('Select a different profile');
    await expect(buttons.nth(2)).toContainText('Reconfigure this profile');
    await expect(buttons.nth(3)).toContainText('Exit');
  });

  test('recovery → Retry: re-attempts connection and dismisses on success', async ({ page }) => {
    let connectAttempts = 0;

    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      load_configuration: BASE_CONFIG,
      list_serial_ports: [],
    });

    // First attempt fails, second succeeds
    await page.addInitScript(() => {
      let attempts = 0;
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'connect_serial') {
          attempts++;
          if (attempts === 1) return Promise.reject('Device or resource busy');
          return Promise.resolve({
            port: '/dev/cu.usbserial-1420',
            baudRate: 38400,
            frequencyHz: 14070000,
            mode: 'DATA-USB',
            connected: true,
          });
        }
        return orig(cmd, args);
      };
    });

    await page.goto('/');

    // Recovery dialog appears after first failure
    await expect(page.locator('.startup-overlay')).toBeVisible({ timeout: 5000 });

    // Click Retry
    await page.locator('.startup-retry-btn').click();

    // Dialog should dismiss after successful retry
    await expect(page.locator('.startup-overlay')).not.toBeVisible({ timeout: 5000 });
  });

  test('no port configured: recovery dialog shown immediately', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      load_configuration: {
        ...BASE_CONFIG,
        serial_port: null,
      },
      list_serial_ports: [],
    });

    await page.goto('/');

    // Recovery dialog must appear
    await expect(page.locator('.startup-overlay')).toBeVisible({ timeout: 5000 });
    await expect(page.locator('.startup-body')).toContainText('No serial port configured');
  });

  test('recovery → Reconfigure: clicking opens settings on Radio tab', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      load_configuration: BASE_CONFIG,
      list_configurations: ['Default'],
      list_audio_devices: [],
      list_serial_ports: [],
    });

    // Override connect_serial to reject
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'connect_serial') return Promise.reject('Port not found');
        return orig(cmd, args);
      };
    });

    await page.goto('/');

    // Wait for recovery dialog
    await expect(page.locator('.startup-overlay')).toBeVisible({ timeout: 5000 });

    // Click "Reconfigure this profile" (nth(2) — after Retry and Select profile)
    await page.locator('.startup-recovery-btn').nth(2).click();

    // Recovery dialog should be dismissed
    await expect(page.locator('.startup-overlay')).not.toBeVisible();

    // Settings dialog should be open on Radio tab
    await expect(page.locator('.settings-overlay')).toHaveClass(/settings-visible/);
    const activeTab = page.locator('.settings-tab.active');
    await expect(activeTab).toHaveAttribute('data-tab', 'radio');
  });

  test('recovery → Select profile: clicking opens settings on General tab', async ({ page }) => {
    await mockInvoke(page, {
      get_connection_status: {
        serialConnected: false, serialPort: null, audioStreaming: false, audioDevice: null,
      },
      load_configuration: BASE_CONFIG,
      list_configurations: ['Default'],
      list_audio_devices: [],
      list_serial_ports: [],
    });

    // Override connect_serial to reject
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'connect_serial') return Promise.reject('Port not found');
        return orig(cmd, args);
      };
    });

    await page.goto('/');

    // Wait for recovery dialog
    await expect(page.locator('.startup-overlay')).toBeVisible({ timeout: 5000 });

    // Click "Select a different profile" (nth(1) — after Retry)
    await page.locator('.startup-recovery-btn').nth(1).click();

    // Recovery dialog should be dismissed
    await expect(page.locator('.startup-overlay')).not.toBeVisible();

    // Settings dialog should be open on General tab
    await expect(page.locator('.settings-overlay')).toHaveClass(/settings-visible/);
    const activeTab = page.locator('.settings-tab.active');
    await expect(activeTab).toHaveAttribute('data-tab', 'general');
  });
});
