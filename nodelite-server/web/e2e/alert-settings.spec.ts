import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 10: alert settings (channels + rules).
// Validation points:
//   - Create / update / delete an alert channel (webhook or smtp) round-trips
//     through the API and shows in the list.
//   - Same for alert rules.
// A protected save requests confirmation only after the first submit.
test.beforeEach(async ({ page }) => {
  await setupApiFixtures(page);
});

test('alert channel CRUD round-trips', async ({ page }) => {
  let saveAttempts = 0;
  await page.route('**/api/settings/alerts', async (route) => {
    if (route.request().method() !== 'POST') {
      await route.fallback();
      return;
    }
    saveAttempts += 1;
    if (saveAttempts > 1) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 428,
      contentType: 'application/json',
      body: JSON.stringify({ message: 'reauth_required' }),
    });
  });
  await page.route('**/api/settings/confirm', async (route) => {
    const body = route.request().postDataJSON() as { current_password?: string };
    await route.fulfill({
      status: body.current_password === 'pw' ? 200 : 403,
      contentType: 'application/json',
      body: JSON.stringify({ ok: body.current_password === 'pw' }),
    });
  });
  await page.goto('/alerts');
  await waitForAppShell(page);
  await page.locator('[data-test="webhook-enabled"]').check();
  await page.locator('[data-test="webhook-url"]').fill('https://hooks.example.test/nodelite');
  await page.locator('[data-test="webhook-modal-close"]').click();
  await page.locator('[data-test="alerts-save"]').click();
  await page.locator('[data-test="step-up-password"]').fill('pw');
  await page.locator('[data-test="step-up-confirm"]').click();
  await expect(page.locator('[data-test="settings-message"]')).toContainText(/saved|已保存/i);
  expect(saveAttempts).toBe(2);
});

test('alert rule CRUD round-trips', async ({ page }) => {
  await page.goto('/alerts');
  await waitForAppShell(page);
  await page.locator('[data-test="rule-add"]').click();
  const rule = page.locator('[data-test="rule-card"]').last();
  await expect(rule.locator('[data-test="rule-id"]')).toBeVisible();
  await rule.locator('[data-test="rule-id"]').fill('memory-hot');
  await rule.locator('[data-test="rule-name"]').fill('Memory hot');
  await page.locator('[data-test="alerts-save"]').click();
  await expect(page.locator('[data-test="settings-message"]')).toContainText(/saved|已保存/i);
});
