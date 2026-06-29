import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 7: node detail tabs.
// Validation points:
//   - overview / monitor / network / logs tabs each switch view and load data.
//   - Each tab finishes loading without console errors.
test.beforeEach(async ({ page }) => {
  await setupApiFixtures(page);
});

test('overview tab loads', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await expect(page.locator('[data-test="node-detail-view"]')).toContainText('Node A');
  await expect(page.locator('[data-test="node-combined-overview"]')).toBeVisible();
});

test('monitor tab loads charts', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await expect(page.locator('[data-test="metric-chart-svg"]').first()).toBeVisible();
});

test('network tab loads interface stats', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await page.locator('[data-test="tab-network"]').click();
  await expect(page.locator('[data-test="network-pane"]')).toBeVisible();
  await expect(page.locator('[data-test="network-traffic-card"]')).toBeVisible();
});

test('logs tab streams entries', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await page.locator('[data-test="tab-logs"]').click();
  await expect(page.locator('[data-test="log-panel"]')).toBeVisible();
  await expect(page.locator('[data-test="log-entry"]')).toContainText('collector started');
});
