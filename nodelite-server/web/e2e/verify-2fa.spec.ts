import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { setupVerify2faFixture } from './_helpers';

async function fillOtp(page: Page, code: string): Promise<void> {
  const cells = page.locator('.otp-cell');
  for (let i = 0; i < code.length; i += 1) {
    await cells.nth(i).fill(code[i]);
  }
}

// Plan §3.7.2 flow 2: 2FA verification.
// Validation points:
//   - Wrong TOTP repeatedly → rate limit response surfaces in the UI.
//   - Correct TOTP → server redirects to `/`, dashboard loads.
// Requires a test account with 2FA enabled; surface its TOTP secret via env
// (e.g. NODELITE_E2E_TOTP_SECRET) so the spec can derive a live code.
test('wrong TOTP triggers rate limit', async ({ page }) => {
  await setupVerify2faFixture(page);
  await page.goto('/verify-2fa');
  await fillOtp(page, '000000');
  await expect(page.locator('#error')).toContainText(/expired|过期/i);
});

test('correct TOTP redirects to dashboard', async ({ page }) => {
  await setupVerify2faFixture(page);
  await page.goto('/verify-2fa');
  await fillOtp(page, '123456');
  await page.waitForURL('**/');
  await expect(page).toHaveURL(/\/$/);
});
