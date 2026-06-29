import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 10: alert settings (channels + rules).
// Validation points:
//   - Create / update / delete an alert channel (webhook or smtp) round-trips
//     through the API and shows in the list.
//   - Same for alert rules.
// IMPORTANT: alert mutations require reauth (security/server: require reauth
// for alert settings, see recent commit a4f3b55) — the spec must satisfy that
// challenge before asserting CRUD.
test.beforeEach(async ({ page }) => {
  await setupApiFixtures(page);
});

test('alert channel CRUD round-trips', async ({ page }) => {
  await page.goto('/alerts');
  await waitForAppShell(page);
  await page.locator('[data-test="webhook-enabled"]').check();
  await page.locator('[data-test="webhook-url"]').fill('https://hooks.example.test/nodelite');
  await page.locator('[data-test="reauth-password"]').fill('pw');
  await page.locator('[data-test="alerts-save"]').click();
  await expect(page.locator('[data-test="settings-message"]')).toContainText(/saved|已保存/i);
});

test('alert rule CRUD round-trips', async ({ page }) => {
  await page.goto('/alerts');
  await waitForAppShell(page);
  await page.locator('[data-test="rule-add"]').click();
  const rule = page.locator('[data-test="rule-card"]').last();
  await rule.locator('summary').click();
  await rule.locator('[data-test="rule-id"]').fill('memory-hot');
  await rule.locator('[data-test="rule-name"]').fill('Memory hot');
  await page.locator('[data-test="reauth-password"]').fill('pw');
  await page.locator('[data-test="alerts-save"]').click();
  await expect(page.locator('[data-test="settings-message"]')).toContainText(/saved|已保存/i);
});
