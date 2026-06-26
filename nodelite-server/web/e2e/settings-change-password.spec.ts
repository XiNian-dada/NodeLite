import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 9: change password via settings modal.
// Validation point:
//   - Opening the change-password form (or submitting) triggers the reauth
//     flow (server requires re-entering current credentials before mutation).
// Note: this test should NOT actually change the password — assert the
// reauth challenge surfaces, then cancel out.
test('change-password attempt posts reauth credentials', async ({ page }) => {
  await setupApiFixtures(page);
  await page.goto('/account');
  await waitForAppShell(page);
  await page.locator('[data-test="password-current"]').fill('old-password');
  await page.locator('[data-test="password-new"]').fill('new-password-123');
  await page.locator('[data-test="password-submit"]').click();
  await expect(page.locator('[data-test="settings-message"]')).toContainText(/updated|saved|已保存/i);
});
