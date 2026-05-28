import { expect, test } from '@playwright/test';

test('legacy dashboard renders after login', async ({ page }) => {
  const response = await page.goto('/');
  expect(response, 'GET / should produce a response').not.toBeNull();
  expect(response!.status(), 'GET / should succeed once credentials are accepted').toBeLessThan(400);

  // Verify the legacy HTML markers — they prove we hit the existing dashboard and not a redirect page.
  await expect(page).toHaveTitle(/NodeLite/i);
  await expect(page.locator('html')).toHaveAttribute('data-refresh-ms', /\d+/);
});
