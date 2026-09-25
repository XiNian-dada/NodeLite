import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 6: node list → detail navigation.
// Validation points:
//   - Clicking a node card navigates to `/nodes/:id`.
//   - WebSocket connection is preserved across the route change (no reconnect
//     marker shown). For the new SPA this is enforced by the App.vue-level
//     singleton; for the legacy UI this is a baseline expectation.
test('click node card navigates to detail', async ({ page }) => {
  await setupApiFixtures(page);
  await page.goto('/');
  await waitForAppShell(page);
  await page.locator('[data-test="node-card"][data-node-id="node-a"]').click();
  await expect(page).toHaveURL(/\/nodes\/node-a$/);
  await expect(page.locator('[data-test="node-detail-view"]')).toContainText('Node A');
});

test('node card leaves room for metrics and the full sparkline', async ({ page }) => {
  await setupApiFixtures(page);
  await page.goto('/');
  await waitForAppShell(page);

  const layout = await page.locator('[data-test="node-card"]').evaluate((card) => {
    const metrics = card.querySelector('.node-metrics');
    const spark = card.querySelector('.node-spark');
    if (!metrics || !spark) throw new Error('node card is missing metrics or sparkline');
    return {
      cardBottom: card.getBoundingClientRect().bottom,
      metricsHeight: metrics.getBoundingClientRect().height,
      sparkBottom: spark.getBoundingClientRect().bottom,
    };
  });

  expect(layout.metricsHeight).toBeGreaterThan(40);
  expect(layout.sparkBottom).toBeLessThanOrEqual(layout.cardBottom);
});
