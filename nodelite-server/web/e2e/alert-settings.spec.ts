import { test } from '@playwright/test';

// Plan §3.7.2 flow 10: alert settings (channels + rules).
// Validation points:
//   - Create / update / delete an alert channel (webhook or smtp) round-trips
//     through the API and shows in the list.
//   - Same for alert rules.
// IMPORTANT: alert mutations require reauth (security/server: require reauth
// for alert settings, see recent commit a4f3b55) — the spec must satisfy that
// challenge before asserting CRUD.
test.fixme('alert channel CRUD round-trips', async ({ page }) => {
  await page.goto('/');
  // TODO: open alert settings, complete reauth, add a test webhook channel,
  // assert it appears, edit it, delete it, assert it's gone.
});

test.fixme('alert rule CRUD round-trips', async () => {
  // TODO: same shape as above but for rules.
});
