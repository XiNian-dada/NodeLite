import { test } from '@playwright/test';

// Plan §3.7.2 flow 4: theme toggle (dark ↔ light).
// Validation points:
//   - Toggling changes the active theme without a visible flash.
//   - Choice persists in localStorage and survives reload.
test.fixme('theme toggle persists across reload', async ({ page }) => {
  await page.goto('/');
  // TODO: locate the theme toggle (legacy: `#theme-toggle`), click,
  // assert `<html data-theme="dark">`, reload, assert state restored.
});
