import { test } from '@playwright/test';

// Plan §3.7.2 flow 9: change password via settings modal.
// Validation point:
//   - Opening the change-password form (or submitting) triggers the reauth
//     flow (server requires re-entering current credentials before mutation).
// Note: this test should NOT actually change the password — assert the
// reauth challenge surfaces, then cancel out.
test.fixme('change-password attempt triggers reauth', async ({ page }) => {
  await page.goto('/');
  // TODO: open settings modal, click "change password", submit a no-op,
  // assert the reauth modal / Basic prompt path is taken.
});
