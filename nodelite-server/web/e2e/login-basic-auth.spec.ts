import { expect, test } from '@playwright/test';

// Plan §3.7.2 flow 1: Login (Basic Auth).
// Validation points:
//   - Visiting `/` without credentials surfaces a 401 (Basic prompt at browser level).
//   - With `NODELITE_E2E_USER`/`PASS` configured, the dashboard finishes loading.
test.fixme('login via Basic Auth lands on the dashboard', async ({ page }) => {
  await page.goto('/');
  await expect(page).toHaveTitle(/NodeLite/i);
  // TODO: assert a stable marker that the dashboard finished hydrating
  // (e.g. node list container becomes visible, no spinner).
});
