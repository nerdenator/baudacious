import { test, expect } from './fixtures';
import { mockInvoke } from './helpers';

/**
 * Phase 3 E2E tests for audio subsystem.
 *
 * These tests mock the Tauri `invoke()` function inline to simulate
 * backend responses without real audio hardware.
 */

test.describe('Audio Panel', () => {
  test('audio input dropdown populates from backend', async ({ page }) => {
    await mockInvoke(page, {
      list_audio_devices: [
        { id: 'FT-991A USB Audio', name: 'FT-991A USB Audio', isInput: true, isDefault: false },
        { id: 'Built-in Microphone', name: 'Built-in Microphone', isInput: true, isDefault: true },
        { id: 'Built-in Speakers', name: 'Built-in Speakers', isInput: false, isDefault: true },
      ],
    });

    await page.goto('/');

    const dropdown = page.locator('#audio-input');
    await expect(dropdown).toBeVisible();

    // Should have placeholder + 2 input devices
    const options = dropdown.locator('option');
    await expect(options).toHaveCount(3);
    await expect(options.nth(1)).toContainText('FT-991A USB Audio');
    await expect(options.nth(2)).toContainText('Built-in Microphone');
  });

  test('default device marked in dropdown', async ({ page }) => {
    await mockInvoke(page, {
      list_audio_devices: [
        { id: 'FT-991A USB Audio', name: 'FT-991A USB Audio', isInput: true, isDefault: false },
        { id: 'Built-in Microphone', name: 'Built-in Microphone', isInput: true, isDefault: true },
      ],
    });

    await page.goto('/');

    const dropdown = page.locator('#audio-input');
    const defaultOption = dropdown.locator('option:has-text("(Default)")');
    await expect(defaultOption).toHaveCount(1);
    await expect(defaultOption).toContainText('Built-in Microphone');
  });

  test('waterfall canvas renders and is visible', async ({ page }) => {
    await mockInvoke(page, { list_audio_devices: [] });
    await page.goto('/');

    const canvas = page.locator('#waterfall-canvas');
    await expect(canvas).toBeVisible();

    const box = await canvas.boundingBox();
    expect(box?.width).toBeGreaterThan(100);
    expect(box?.height).toBeGreaterThan(50);
  });

  test('audio status shows N/C by default', async ({ page }) => {
    await mockInvoke(page, { list_audio_devices: [] });
    await page.goto('/');

    const audioInText = page.locator('#audio-in-status .status-text');
    await expect(audioInText).toHaveText('N/C');

    const audioInDot = page.locator('#audio-in-status .status-dot');
    await expect(audioInDot).toHaveClass(/disconnected/);
  });
});
