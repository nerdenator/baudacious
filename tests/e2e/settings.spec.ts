import { test, expect } from '@playwright/test';
import { mockInvoke, fireEvent } from './helpers';

/**
 * Phase 6 E2E tests for the settings dialog.
 *
 * The dialog is opened by firing a 'menu-event' Tauri event (the same path
 * the native menu bar uses). All backend calls are mocked.
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
};

/** Base mocks used by most settings tests */
const BASE_MOCKS = {
  get_connection_status: {
    serialConnected: false,
    serialPort: null,
    audioStreaming: false,
    audioDevice: null,
  },
  list_configurations: ['Default', 'Contest'],
  load_configuration: DEFAULT_CONFIG,
  list_audio_devices: [
    { id: 'mic-1', name: 'Built-in Microphone', isInput: true, isOutput: false, isDefault: true, outputUnverified: false },
    { id: 'spk-1', name: 'Built-in Speakers', isInput: false, isOutput: true, isDefault: true, outputUnverified: false },
  ],
  save_configuration: null,
  delete_configuration: null,
  list_serial_ports: [],
};

/** Fire the menu-event that opens the settings dialog */
async function openSettings(page: import('@playwright/test').Page) {
  await fireEvent(page, 'menu-event', { id: 'settings' });
  await expect(page.locator('.settings-overlay')).toHaveClass(/settings-visible/);
}

test.describe('Settings Dialog', () => {
  test('dialog is hidden on page load', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await expect(page.locator('.settings-overlay')).not.toHaveClass(/settings-visible/);
  });

  test('opens to general tab via menu event', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);

    const activeTab = page.locator('.settings-tab.active');
    await expect(activeTab).toHaveAttribute('data-tab', 'general');
  });

  test('general tab populates profile list from backend', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);

    const profileSelect = page.locator('.settings-profile-select');
    await expect(profileSelect.locator('option')).toHaveCount(2);
    await expect(profileSelect.locator('option').first()).toHaveText('Default');
    await expect(profileSelect.locator('option').last()).toHaveText('Contest');
  });

  test('audio tab populates device selects from backend', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);
    await page.locator('.settings-tab[data-tab="audio"]').click();

    // Audio panel is section.settings-panel index 1 (general=0, audio=1, radio=2)
    const audioPanel = page.locator('section.settings-panel').nth(1);

    // Input: placeholder + 1 device
    const inputSelect = audioPanel.locator('.device-select').first();
    await expect(inputSelect.locator('option')).toHaveCount(2);
    await expect(inputSelect.locator('option').last()).toContainText('Built-in Microphone (Default)');

    // Output: placeholder + 1 device
    const outputSelect = audioPanel.locator('.device-select').last();
    await expect(outputSelect.locator('option')).toHaveCount(2);
    await expect(outputSelect.locator('option').last()).toContainText('Built-in Speakers (Default)');
  });

  test('switching profile reloads all form fields', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);

    // Override load_configuration to return different data per name
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === 'load_configuration' && args?.name === 'Contest') {
          return Promise.resolve({
            name: 'Contest',
            audio_input: 'mic-1',
            audio_output: 'spk-1',
            serial_port: null,
            baud_rate: 9600,
            radio_type: 'FT-991A',
            carrier_freq: 1000.0,
            waterfall_palette: 'classic',
            waterfall_noise_floor: -100,
            waterfall_zoom: 1,
          });
        }
        return orig(cmd, args);
      };
    });

    await page.goto('/');
    await openSettings(page);

    await page.locator('.settings-profile-select').selectOption('Contest');

    // Profile name input updates to the loaded profile's name
    await expect(page.locator('.settings-input')).toHaveValue('Contest');

    // Baud rate select updates (radio panel, second .device-select)
    const radioPanel = page.locator('section.settings-panel').nth(2);
    await expect(radioPanel.locator('.device-select').last()).toHaveValue('9600');
  });

  test('delete button disabled for Default profile', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);

    await expect(page.locator('.settings-danger-btn')).toBeDisabled();
  });

  test('empty profile name blocks save', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);

    await page.locator('.settings-input').clear();
    await page.locator('.settings-save-btn').click();

    // Dialog must still be visible (save was rejected)
    await expect(page.locator('.settings-overlay')).toHaveClass(/settings-visible/);
  });

  test('save closes dialog and shows info toast', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);
    await page.locator('.settings-save-btn').click();

    // Dialog closes on success
    await expect(page.locator('.settings-overlay')).not.toHaveClass(/settings-visible/);

    // Info toast appears
    await expect(page.locator('.toast.toast-info')).toContainText('Settings saved');
  });

  test('Escape key closes the dialog', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);
    await page.keyboard.press('Escape');

    await expect(page.locator('.settings-overlay')).not.toHaveClass(/settings-visible/);
  });

  test('cancel button closes the dialog', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');

    await openSettings(page);
    await page.locator('.settings-cancel-btn').click();

    await expect(page.locator('.settings-overlay')).not.toHaveClass(/settings-visible/);
  });
});
