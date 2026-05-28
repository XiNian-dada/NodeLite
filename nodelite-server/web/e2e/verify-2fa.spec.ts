import { test } from '@playwright/test';

// Plan §3.7.2 flow 2: 2FA verification.
// Validation points:
//   - Wrong TOTP repeatedly → rate limit response surfaces in the UI.
//   - Correct TOTP → server redirects to `/`, dashboard loads.
// Requires a test account with 2FA enabled; surface its TOTP secret via env
// (e.g. NODELITE_E2E_TOTP_SECRET) so the spec can derive a live code.
test.fixme('wrong TOTP triggers rate limit', async ({ page }) => {
  await page.goto('/verify-2fa');
  // TODO: submit 5 wrong codes, expect the rate-limit banner / 429 surfacing.
});

test.fixme('correct TOTP redirects to dashboard', async ({ page }) => {
  await page.goto('/verify-2fa');
  // TODO: derive TOTP from NODELITE_E2E_TOTP_SECRET, submit, expect URL `/`.
});
