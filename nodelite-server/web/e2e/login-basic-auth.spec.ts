import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 1: Login (Basic Auth).
// Validation points:
//   - Visiting `/` without credentials surfaces a 401 (Basic prompt at browser level).
//   - With `NODELITE_E2E_USER`/`PASS` configured, the dashboard finishes loading.
test('login via Basic Auth lands on the dashboard', async ({ page }) => {
  await setupApiFixtures(page);
  await page.goto('/');
  await waitForAppShell(page);
  await expect(page).toHaveTitle(/NodeLite/i);
  await expect(page.locator('[data-test="dashboard-view"]')).toBeVisible();
  await expect(page.locator('[data-test="node-card"][data-node-id="node-a"]')).toBeVisible();
});
