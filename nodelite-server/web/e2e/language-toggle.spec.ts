import { test } from '@playwright/test';

// Plan §3.7.2 flow 5: language toggle (en ↔ zh).
// Validation point:
//   - Every element with `data-i18n` (legacy) / vue-i18n binding (new) updates
//     to the chosen language. Spot-check a stable set of keys (nav, headers).
test.fixme('language toggle updates all i18n-bound copy', async ({ page }) => {
  await page.goto('/');
  // TODO: switch to zh, assert a few known strings; switch back to en, re-assert.
});
