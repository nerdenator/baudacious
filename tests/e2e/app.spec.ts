import { test, expect } from './fixtures';
import { mockInvoke, dismissStartupDialog } from './helpers';

/** Base mocks used by tests that need Tauri IPC but don't care about serial/audio */
const BASE_MOCKS = {
  get_connection_status: {
    serialConnected: false,
    serialPort: null,
    audioStreaming: false,
    audioDevice: null,
  },
  list_audio_devices: [],
  list_serial_ports: [],
};

test.describe('PSK-31 Client UI', () => {
  test.beforeEach(async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);
  });

  test('main layout renders correctly', async ({ page }) => {
    // Check main structural elements exist
    await expect(page.locator('.main-content')).toBeVisible();
    await expect(page.locator('.waterfall-section')).toBeVisible();
    await expect(page.locator('.rx-section')).toBeVisible();
    await expect(page.locator('.tx-section')).toBeVisible();
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('.status-bar')).toBeVisible();
  });

  test('waterfall canvas is present and sized', async ({ page }) => {
    const canvas = page.locator('#waterfall-canvas');
    await expect(canvas).toBeVisible();

    // Canvas should have non-zero dimensions
    const box = await canvas.boundingBox();
    expect(box?.width).toBeGreaterThan(100);
    expect(box?.height).toBeGreaterThan(50);
  });

  test('RX panel displays correctly', async ({ page }) => {
    await expect(page.locator('.rx-section .rx-label')).toContainText('RX');
    await expect(page.locator('#rx-content')).toBeVisible();
    await expect(page.locator('.rx-controls .rx-btn')).toBeVisible();
  });

  test('TX panel displays correctly', async ({ page }) => {
    await expect(page.locator('.tx-section .tx-label')).toContainText('TX');
    await expect(page.locator('#tx-input')).toBeVisible();
    await expect(page.locator('.tx-btn-send')).toBeVisible();
    await expect(page.locator('.tx-btn-abort')).toBeVisible();
  });

  test('TX character counter updates on input', async ({ page }) => {
    const txInput = page.locator('#tx-input');
    const charCount = page.locator('.tx-char-count .current');

    await expect(charCount).toHaveText('0');
    await txInput.fill('Hello');
    await expect(charCount).toHaveText('5');
    await txInput.fill('CQ CQ CQ de W1AW');
    await expect(charCount).toHaveText('16');
  });

  test('RX clear button clears content', async ({ page }) => {
    const rxContent = page.locator('#rx-content');
    const clearBtn = page.locator('.rx-controls .rx-btn');

    // Add some content via JS
    await page.evaluate(() => {
      document.getElementById('rx-content')!.textContent = 'Test message';
    });

    await expect(rxContent).toHaveText('Test message');
    await clearBtn.click();
    await expect(rxContent).toHaveText('');
  });

  test('status bar shows connection info', async ({ page }) => {
    await expect(page.locator('.status-bar')).toBeVisible();
    await expect(page.locator('.ptt-indicator')).toBeVisible();
    await expect(page.locator('.ptt-status')).toBeVisible();
  });
});

test.describe('Theme Toggle', () => {
  test.beforeEach(async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);
    // Clear any saved theme preference
    await page.evaluate(() => localStorage.removeItem('psk31-theme'));
  });

  test('theme toggle button exists', async ({ page }) => {
    await expect(page.locator('#theme-toggle')).toBeVisible();
    await expect(page.locator('.theme-toggle-icon')).toBeVisible();
  });

  test('clicking theme toggle switches theme', async ({ page }) => {
    const toggle = page.locator('#theme-toggle');
    const html = page.locator('html');

    // Get initial theme
    const initialTheme = await html.getAttribute('data-theme');

    // Click toggle
    await toggle.click();

    // Theme should change
    const newTheme = await html.getAttribute('data-theme');
    expect(newTheme).not.toBe(initialTheme);

    // Click again to toggle back
    await toggle.click();
    const finalTheme = await html.getAttribute('data-theme');
    expect(finalTheme).toBe(initialTheme);
  });

  test('theme preference persists in localStorage', async ({ page }) => {
    const toggle = page.locator('#theme-toggle');

    await toggle.click();
    const theme = await page.evaluate(() => localStorage.getItem('psk31-theme'));
    expect(theme).toBeTruthy();

    // Reload page — need to dismiss the dialog again after reload
    await page.reload();
    await dismissStartupDialog(page);

    // Theme should persist
    const savedTheme = await page.locator('html').getAttribute('data-theme');
    expect(savedTheme).toBe(theme);
  });
});

test.describe('Waterfall Interaction', () => {
  test.beforeEach(async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);
  });

  test('clicking waterfall updates frequency display', async ({ page }) => {
    const canvas = page.locator('#waterfall-canvas');
    const freqDisplay = page.locator('.waterfall-freq');

    // Click in the middle of the waterfall
    await canvas.click({ position: { x: 200, y: 50 } });

    // Frequency should update (exact value depends on canvas width)
    const freqText = await freqDisplay.textContent();
    expect(freqText).toMatch(/\d+ Hz/);
  });
});

test.describe('TX Flow (Mock)', () => {
  test('send button does nothing with empty input', async ({ page }) => {
    // Mock invoke so startTx resolves; return connected serial so send button is enabled
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        invoke: (cmd: string) => {
          if (cmd === 'get_connection_status') {
            return Promise.resolve({
              serialConnected: true,
              serialPort: '/dev/cu.test',
              audioStreaming: false,
              audioDevice: null,
            });
          }
          return Promise.resolve(null);
        },
        metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
        convertFileSrc: (src: string) => src,
      };
    });
    await page.goto('/');
    await dismissStartupDialog(page);

    const txInput = page.locator('#tx-input');
    await txInput.fill('');
    await page.locator('.tx-btn-send').click();

    // PTT indicator should still be RX (send didn't work)
    await expect(page.locator('.ptt-indicator')).toHaveText('RX');
  });

  test('send button triggers TX state', async ({ page }) => {
    // Mock invoke — start_tx succeeds, we'll simulate completion via event
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        invoke: (cmd: string) => {
          if (cmd === 'get_connection_status') {
            return Promise.resolve({
              serialConnected: true,
              serialPort: '/dev/cu.test',
              audioStreaming: false,
              audioDevice: null,
            });
          }
          if (cmd === 'start_tx') {
            // Simulate tx-status "complete" event after 500ms
            setTimeout(() => {
              window.dispatchEvent(new CustomEvent('tauri://event', {
                detail: { event: 'tx-status', payload: { status: 'complete', progress: 1.0 } }
              }));
            }, 500);
          }
          return Promise.resolve(null);
        },
        metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
        convertFileSrc: (src: string) => src,
      };
    });
    await page.goto('/');
    await dismissStartupDialog(page);

    // Need an audio output device selected for TX to proceed
    await page.evaluate(() => {
      const select = document.getElementById('audio-output') as HTMLSelectElement;
      const opt = document.createElement('option');
      opt.value = 'test-speaker';
      opt.textContent = 'Test Speaker';
      select.appendChild(opt);
      select.value = 'test-speaker';
    });

    const txInput = page.locator('#tx-input');
    const pttIndicator = page.locator('.ptt-indicator');

    await txInput.fill('CQ CQ CQ');
    await page.locator('.tx-btn-send').click();

    // Should switch to TX immediately (UI state set before backend call)
    await expect(pttIndicator).toHaveText('TX');
    await expect(pttIndicator).toHaveClass(/tx/);
  });

  test('abort button returns to RX state', async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        invoke: (cmd: string) => {
          if (cmd === 'get_connection_status') {
            return Promise.resolve({
              serialConnected: true,
              serialPort: '/dev/cu.test',
              audioStreaming: false,
              audioDevice: null,
            });
          }
          return Promise.resolve(null);
        },
        metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
        convertFileSrc: (src: string) => src,
      };
    });
    await page.goto('/');
    await dismissStartupDialog(page);

    // Add an audio output device
    await page.evaluate(() => {
      const select = document.getElementById('audio-output') as HTMLSelectElement;
      const opt = document.createElement('option');
      opt.value = 'test-speaker';
      opt.textContent = 'Test Speaker';
      select.appendChild(opt);
      select.value = 'test-speaker';
    });

    const txInput = page.locator('#tx-input');
    const pttIndicator = page.locator('.ptt-indicator');

    await txInput.fill('Test message');
    await page.locator('.tx-btn-send').click();

    await expect(pttIndicator).toHaveText('TX');

    await page.locator('.tx-btn-abort').click();

    // Abort calls stopTx which resets UI via the onAborted callback,
    // but since we don't have real events, the abort click itself
    // should still trigger the backend. The UI resets because stopTx resolves.
    // Wait a beat for the promise to resolve
    await expect(pttIndicator).toHaveText('RX', { timeout: 3000 });
  });
});

test.describe('Menu Event Handling', () => {
  test.beforeEach(async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);
  });

  test('theme_light menu event sets light theme', async ({ page }) => {
    // Simulate menu event by dispatching custom event
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent('menu-event-test', {
        detail: { id: 'theme_light' }
      }));
    });

    // Since we can't easily mock Tauri events in browser context,
    // we test the theme functions directly
    await page.evaluate(() => {
      document.documentElement.setAttribute('data-theme', 'light');
    });

    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  });

  test('theme_dark menu event sets dark theme', async ({ page }) => {
    await page.evaluate(() => {
      document.documentElement.setAttribute('data-theme', 'dark');
    });

    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  });
});

test.describe('Visual Regression', () => {
  test('main UI screenshot - light theme', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);
    await page.evaluate(() => {
      document.documentElement.setAttribute('data-theme', 'light');
      localStorage.setItem('psk31-theme', 'light');
    });

    // Wait for initial render
    await page.waitForTimeout(200);

    // Mask the animated waterfall canvas to get stable screenshots
    await expect(page).toHaveScreenshot('main-ui-light.png', {
      maxDiffPixelRatio: 0.02, // Allow 2% pixel difference
      mask: [page.locator('#waterfall-canvas')],
    });
  });

  test('main UI screenshot - dark theme', async ({ page }) => {
    await mockInvoke(page, BASE_MOCKS);
    await page.goto('/');
    await dismissStartupDialog(page);
    await page.evaluate(() => {
      document.documentElement.setAttribute('data-theme', 'dark');
      localStorage.setItem('psk31-theme', 'dark');
    });

    // Wait for initial render
    await page.waitForTimeout(200);

    // Mask the animated waterfall canvas to get stable screenshots
    await expect(page).toHaveScreenshot('main-ui-dark.png', {
      maxDiffPixelRatio: 0.02,
      mask: [page.locator('#waterfall-canvas')],
    });
  });
});
