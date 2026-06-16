import { expect, test } from '@playwright/test';

// Plan §3.7.2 flow 3: 24h auto-logout.
// Validation point: when the cached auth timestamp is older than 24h,
// the SPA's inline expiry shim (index.html) redirects to /logout-and-reauth
// before the Vue app even mounts.
test('client triggers logout-and-reauth after 24h', async ({ page }) => {
  await page.addInitScript(() => {
    const TWENTY_FIVE_HOURS = 25 * 60 * 60 * 1000;
    window.localStorage.setItem(
      'nodelite.auth.timestamp',
      String(Date.now() - TWENTY_FIVE_HOURS),
    );
  });

  await page.goto('/', { waitUntil: 'commit' });
  await page.waitForURL('**/logout-and-reauth', { timeout: 5000 });
  await expect(page).toHaveURL(/\/logout-and-reauth$/);
});

test('client triggers logout-and-reauth when auth storage is unavailable', async ({ page }) => {
  await page.addInitScript(() => {
    const storageBlocked = () => {
      throw new Error('storage blocked');
    };

    Storage.prototype.getItem = storageBlocked;
    Storage.prototype.setItem = storageBlocked;
    Storage.prototype.removeItem = storageBlocked;
  });

  await page.goto('/', { waitUntil: 'commit' });
  await page.waitForURL('**/logout-and-reauth', { timeout: 5000 });
  await expect(page).toHaveURL(/\/logout-and-reauth$/);
});
