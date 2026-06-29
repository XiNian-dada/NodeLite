import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 8: chart interaction.
// Validation points (per §3.7.4 we do NOT do pixel diffing):
//   - Hovering a chart shows a tooltip with readable text.
//   - Clicking a chart opens its modal (if applicable) and the modal can close.
//   - Zoom / brush gestures (if present) reflect in tooltip values.
test.beforeEach(async ({ page }) => {
  await setupApiFixtures(page);
});

test('hover surfaces a tooltip', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  const chart = page.locator('[data-test="metric-chart"]').first();
  await chart.hover({ position: { x: 180, y: 90 } });
  await expect(page.locator('[data-test="metric-chart-tooltip"]').first()).toBeVisible();
});

test('chart modal opens and closes', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await page.locator('[data-test="zoom-cpu"]').click();
  await expect(page.locator('[data-test="chart-modal"]')).toBeVisible();
  await page.locator('[data-test="chart-modal-close"]').click();
  await expect(page.locator('[data-test="chart-modal"]')).toBeHidden();
});
