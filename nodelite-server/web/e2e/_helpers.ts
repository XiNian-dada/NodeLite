import type { Page } from '@playwright/test';

/**
 * Wait for App.vue to finish its async mount (setupI18n awaits a 39 KB
 * dictionary fetch before mounting). page.goto() only resolves on the
 * `load` event — which fires before Vue's mount completes — so any spec
 * that touches the SPA UI must wait for this marker first.
 *
 * The selector is set on App.vue's root element.
 */
export async function waitForAppShell(page: Page): Promise<void> {
  await page.waitForSelector('[data-test="app-shell"]', { state: 'attached' });
}
