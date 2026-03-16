import { test as base, expect } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

/**
 * Extended Playwright test fixture that automatically collects Istanbul
 * coverage data (window.__coverage__) after each test and writes it to
 * .nyc_output/ for aggregation by nyc.
 *
 * Only active when the app is built with VITE_COVERAGE=true.
 */
export const test = base.extend<{ autoCollectCoverage: void }>({
  autoCollectCoverage: [async ({ page }, use) => {
    await use();
    try {
      const coverage = await page.evaluate(() => (window as any).__coverage__);
      if (coverage) {
        const dir = path.resolve('.nyc_output');
        fs.mkdirSync(dir, { recursive: true });
        const filename = `coverage-${Date.now()}-${Math.random().toString(36).slice(2)}.json`;
        fs.writeFileSync(path.join(dir, filename), JSON.stringify(coverage));
      }
    } catch {
      // Page may be closed or coverage instrumentation not active
    }
  }, { auto: true }],
});

export { expect };
