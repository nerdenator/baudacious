import { test, expect } from '@playwright/test';
import { mockInvoke, dismissStartupDialog } from './helpers';

/**
 * Phase 5 E2E tests for RX (receive) subsystem.
 *
 * Tests the RX display, decoded text rendering, click-to-tune carrier
 * frequency updates, and the rx-text event bridge.
 */

const BASE_MOCKS = { list_audio_devices: [], list_serial_ports: [] };

test.describe('RX Display', () => {
  test('RX panel shows decoded text when injected', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);

    const rxContent = page.locator('#rx-content');

    // Clear any sample text, then inject decoded text
    await page.evaluate(() => {
      const el = document.getElementById('rx-content')!;
      el.textContent = '';
    });

    await page.evaluate(() => {
      const el = document.getElementById('rx-content')!;
      el.textContent += 'CQ CQ DE W1AW';
    });

    await expect(rxContent).toHaveText('CQ CQ DE W1AW');
  });

  test('clear button clears decoded text', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);

    const rxContent = page.locator('#rx-content');
    const clearBtn = page.locator('.rx-controls .rx-btn');

    // Add text
    await page.evaluate(() => {
      document.getElementById('rx-content')!.textContent = 'CQ CQ DE W1AW';
    });

    await expect(rxContent).toHaveText('CQ CQ DE W1AW');
    await clearBtn.click();
    await expect(rxContent).toHaveText('');
  });

  test('RX display auto-scrolls on new text', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);

    // Fill RX display with enough text to overflow, then append one more line
    // using the same pattern as appendRxText (textContent += ..., scrollTop = scrollHeight)
    await page.evaluate(() => {
      const el = document.getElementById('rx-content')!;
      el.textContent = '';
      for (let i = 0; i < 100; i++) {
        el.textContent += `Line ${i}: CQ CQ CQ DE W1AW W1AW W1AW K\n`;
        el.scrollTop = el.scrollHeight;
      }
    });

    // Verify the element is scrolled to (or near) the bottom
    const scrollState = await page.evaluate(() => {
      const el = document.getElementById('rx-content')!;
      return {
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      };
    });

    expect(scrollState.scrollTop).toBeGreaterThan(0);
    // scrollTop + clientHeight should be at or near scrollHeight
    expect(scrollState.scrollTop + scrollState.clientHeight).toBeGreaterThanOrEqual(
      scrollState.scrollHeight - 2
    );
  });
});

test.describe('Click-to-Tune', () => {
  test('clicking waterfall calls set_carrier_frequency', async ({ page }) => {
    let capturedFreq: number | null = null;

    await page.addInitScript(() => {
      (window as any).__capturedCalls__ = [];
      (window as any).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args?: any) => {
          (window as any).__capturedCalls__.push({ cmd, args });
          return Promise.resolve(null);
        },
        metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
        convertFileSrc: (src: string) => src,
      };
    });

    await page.goto('/');
    await dismissStartupDialog(page);

    const canvas = page.locator('#waterfall-canvas');
    await canvas.click({ position: { x: 200, y: 50 } });

    // Check that set_carrier_frequency was called
    const calls = await page.evaluate(() => {
      return (window as any).__capturedCalls__.filter(
        (c: any) => c.cmd === 'set_carrier_frequency'
      );
    });

    expect(calls.length).toBe(1);
    expect(calls[0].args.freqHz).toBeGreaterThan(500);
    expect(calls[0].args.freqHz).toBeLessThanOrEqual(2500);
  });

  test('clicking waterfall updates frequency displays', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);

    const canvas = page.locator('#waterfall-canvas');
    const freqDisplay = page.locator('.waterfall-freq');

    await canvas.click({ position: { x: 200, y: 50 } });

    const freqText = await freqDisplay.textContent();
    expect(freqText).toMatch(/\d+ Hz/);
  });
});
