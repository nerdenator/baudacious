import { test, expect } from '@playwright/test';
import { mockInvoke } from './helpers';

/**
 * Phase 2 E2E tests for serial/CAT communication.
 *
 * These tests mock the Tauri `invoke()` function inline to simulate
 * backend responses without a real serial connection.
 */

test.describe('Serial Panel', () => {
  test('serial port dropdown populates from backend', async ({ page }) => {
    await mockInvoke(page, {
      list_serial_ports: [
        { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)' },
        { name: '/dev/cu.usbserial-1430', portType: 'USB (10C4:EA60)' },
      ],
    });

    await page.goto('/');

    const dropdown = page.locator('#serial-port');
    await expect(dropdown).toBeVisible();

    // Should have placeholder + 2 ports
    const options = dropdown.locator('option');
    await expect(options).toHaveCount(3);
    await expect(options.nth(1)).toContainText('/dev/cu.usbserial-1420');
    await expect(options.nth(2)).toContainText('/dev/cu.usbserial-1430');
  });

  test('connect button updates frequency display on success', async ({ page }) => {
    await mockInvoke(page, {
      list_serial_ports: [
        { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)' },
      ],
      connect_serial: {
        port: '/dev/cu.usbserial-1420',
        baudRate: 38400,
        frequencyHz: 14070000,
        mode: 'DATA-USB',
        connected: true,
      },
    });

    await page.goto('/');

    // Select port and click connect
    const dropdown = page.locator('#serial-port');
    await dropdown.selectOption('/dev/cu.usbserial-1420');
    const connectBtn = page.locator('#serial-connect-btn');
    await connectBtn.click();

    // Frequency and band should update
    await expect(page.locator('#freq-mhz-input')).toHaveValue('14.070');
    await expect(page.locator('#band-select')).toHaveValue('20m');

    // Mode should update
    const freqMode = page.locator('.frequency-mode');
    await expect(freqMode).toHaveText('DATA-USB');
  });

  test('CAT status indicator shows connected after connect', async ({ page }) => {
    await mockInvoke(page, {
      list_serial_ports: [
        { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)' },
      ],
      connect_serial: {
        port: '/dev/cu.usbserial-1420',
        baudRate: 38400,
        frequencyHz: 14070000,
        mode: 'DATA-USB',
        connected: true,
      },
    });

    await page.goto('/');

    const dropdown = page.locator('#serial-port');
    await dropdown.selectOption('/dev/cu.usbserial-1420');
    await page.locator('#serial-connect-btn').click();

    // CAT status dot should be connected
    const catDot = page.locator('#cat-status .status-dot');
    await expect(catDot).toHaveClass(/connected/);

    // CAT status text should say OK
    const catText = page.locator('#cat-status .status-text');
    await expect(catText).toHaveText('OK');
  });

  test('disconnect button resets UI', async ({ page }) => {
    await mockInvoke(page, {
      list_serial_ports: [
        { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)' },
      ],
      connect_serial: {
        port: '/dev/cu.usbserial-1420',
        baudRate: 38400,
        frequencyHz: 14070000,
        mode: 'DATA-USB',
        connected: true,
      },
      disconnect_serial: null,
    });

    await page.goto('/');

    // Connect first
    const dropdown = page.locator('#serial-port');
    await dropdown.selectOption('/dev/cu.usbserial-1420');
    const connectBtn = page.locator('#serial-connect-btn');
    await connectBtn.click();

    // Wait for green flash to settle, then text becomes "Disconnect"
    await expect(connectBtn).toHaveText('Disconnect', { timeout: 12000 });

    // Click disconnect
    await connectBtn.click();

    // CAT status should be disconnected
    const catText = page.locator('#cat-status .status-text');
    await expect(catText).toHaveText('N/C');

    // Dropdown should be re-enabled
    await expect(dropdown).toBeEnabled();

    // Button should say Connect again
    await expect(connectBtn).toHaveText('Connect');
  });

  test('error message displays on connection failure', async ({ page }) => {
    await mockInvoke(page, {
      list_serial_ports: [
        { name: '/dev/cu.usbserial-1420', portType: 'USB (10C4:EA60)' },
      ],
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

    const dropdown = page.locator('#serial-port');
    await dropdown.selectOption('/dev/cu.usbserial-1420');
    await page.locator('#serial-connect-btn').click();

    // CAT status should show error
    const catText = page.locator('#cat-status .status-text');
    await expect(catText).toHaveText('Error');

    // Button should be re-enabled for retry
    const connectBtn = page.locator('#serial-connect-btn');
    await expect(connectBtn).toBeEnabled();
    await expect(connectBtn).toHaveText('Connect');
  });
});
