import { expect, test } from '@playwright/test';
import { waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 4: theme toggle (dark ↔ light).
// Validation points:
//   - Toggling changes the active theme without a visible flash.
//   - Choice persists in localStorage and survives reload.
test('theme toggle persists across reload', async ({ page }) => {
  await page.goto('/');
  await waitForAppShell(page);

  const initial = await page.locator('html').getAttribute('data-theme');
  await page.locator('[data-test="theme-toggle"]').click();

  const next = initial === 'dark' ? 'light' : 'dark';
  await expect(page.locator('html')).toHaveAttribute('data-theme', next);

  await page.reload();
  await waitForAppShell(page);
  await expect(page.locator('html')).toHaveAttribute('data-theme', next);
});
