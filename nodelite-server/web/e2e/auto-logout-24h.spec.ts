import { test } from '@playwright/test';

// Plan §3.7.2 flow 3: 24h auto-logout.
// Validation point:
//   - Mock the timestamp the client uses to decide it's been 24h since auth →
//     UI navigates to `/logout-and-reauth` (clearing the 2FA cookie).
// Note: legacy UI reads the timestamp from inline JS; for the new SPA this
// will likely be a store value driven by `bootstrap.refresh_interval_secs`
// or a dedicated session-expiry timer.
test.fixme('client triggers logout-and-reauth after 24h', async ({ page }) => {
  await page.goto('/');
  // TODO: page.clock.install() once we're on Playwright 1.45+, fast-forward 24h,
  // assert URL becomes `/logout-and-reauth`.
});
