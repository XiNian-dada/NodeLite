import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

test('dashboard renders after login', async ({ page }) => {
  await setupApiFixtures(page);
  const response = await page.goto('/');
  expect(response, 'GET / should produce a response').not.toBeNull();
  expect(response!.status(), 'GET / should succeed once credentials are accepted').toBeLessThan(400);

  await waitForAppShell(page);
  await expect(page).toHaveTitle(/NodeLite/i);
  await expect(page.locator('[data-test="dashboard-view"]')).toBeVisible();
});
