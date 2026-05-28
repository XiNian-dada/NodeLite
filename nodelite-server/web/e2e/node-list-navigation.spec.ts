import { test } from '@playwright/test';

// Plan §3.7.2 flow 6: node list → detail navigation.
// Validation points:
//   - Clicking a node card navigates to `/nodes/:id`.
//   - WebSocket connection is preserved across the route change (no reconnect
//     marker shown). For the new SPA this is enforced by the App.vue-level
//     singleton; for the legacy UI this is a baseline expectation.
test.fixme('click node card navigates to detail with live WS', async ({ page }) => {
  await page.goto('/');
  // TODO: wait for a node card, capture its ID, click, assert URL `/nodes/<id>`,
  // assert WS status indicator stays in the "connected" state.
});
