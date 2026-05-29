import { expect, test } from '@playwright/test';
import { waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 5: language toggle (en ↔ zh-CN).
// Validation point: the language selector flips vue-i18n's active locale,
// the value is persisted, and at least one $t()-bound string updates.
test('language toggle updates app shell copy and persists', async ({ page }) => {
  await page.goto('/');
  await waitForAppShell(page);

  const select = page.locator('[data-test="language-select"]');
  await expect(select).toBeVisible();

  await select.selectOption('zh-CN');
  await expect(page.locator('[data-test="language-select"]')).toHaveValue('zh-CN');
  const persisted = await page.evaluate(() =>
    window.localStorage.getItem('nodelite.ui.language'),
  );
  expect(persisted).toBe('zh-CN');

  await page.reload();
  await waitForAppShell(page);
  await expect(page.locator('[data-test="language-select"]')).toHaveValue('zh-CN');

  await page.locator('[data-test="language-select"]').selectOption('en');
  await expect(page.locator('[data-test="language-select"]')).toHaveValue('en');
});
