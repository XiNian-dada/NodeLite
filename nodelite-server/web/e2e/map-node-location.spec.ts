import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 11: map node location.
// Validation point:
//   - Clicking a node marker on the world map highlights the corresponding
//     node card and navigates to its detail page.
test('map marker exposes the node and card navigates to detail', async ({ page }) => {
  await setupApiFixtures(page);
  await page.goto('/');
  await waitForAppShell(page);
  await page.locator('[data-test="map-dot"]').first().hover();
  await expect(page.locator('[data-test="map-hover-card"]')).toContainText('Node A');
  await page.locator('[data-test="node-card"][data-node-id="node-a"]').click();
  await expect(page).toHaveURL(/\/nodes\/node-a$/);
});
