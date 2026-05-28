import { test } from '@playwright/test';

// Plan §3.7.2 flow 12: WebSocket disconnect / reconnect.
// Validation points:
//   - When the WS drops (we simulate via `page.route` blocking `/ws` or
//     `context.setOffline(true)`), the UI surfaces a reconnect indicator.
//   - Once connectivity is restored, the indicator clears and live data resumes.
test.fixme('WS drop surfaces reconnect UI then recovers', async ({ page, context }) => {
  await page.goto('/');
  // TODO: wait for WS to be established, drop it, assert reconnect banner,
  // restore network, assert banner clears and a fresh tick lands.
  void context;
});
