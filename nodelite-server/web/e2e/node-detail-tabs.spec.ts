import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 7: node detail tabs.
// Validation points:
//   - overview / monitor / network / logs tabs each switch view and load data.
//   - Each tab finishes loading without console errors.
test.beforeEach(async ({ page }) => {
  await setupApiFixtures(page);
});

test('overview tab loads', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await expect(page.locator('[data-test="node-detail-view"]')).toContainText('Node A');
  await expect(page.locator('[data-test="node-combined-overview"]')).toBeVisible();
});

test('monitor tab loads charts', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await expect(page.locator('[data-test="metric-chart-svg"]').first()).toBeVisible();
});

test('network tab loads interface stats', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await page.locator('[data-test="tab-network"]').click();
  await expect(page.locator('[data-test="network-pane"]')).toBeVisible();
  await expect(page.locator('[data-test="network-traffic-card"]')).toBeVisible();
});

test('long disk mounts stay within their column across the mobile breakpoint', async ({ page }) => {
  await page.setViewportSize({ width: 561, height: 900 });
  await page.goto('/nodes/node-a#hardware');
  await waitForAppShell(page);

  const mount = page.locator('[data-test="disk-mount"]');
  const filesystem = mount.locator('xpath=following-sibling::span[1]');

  await expect(mount).toBeVisible();
  await expect(mount).toHaveCSS('overflow', 'hidden');
  await expect(mount).toHaveCSS('text-overflow', 'ellipsis');
  await expect(mount).toHaveCSS('white-space', 'nowrap');

  const desktopMountBox = await mount.boundingBox();
  const filesystemBox = await filesystem.boundingBox();
  expect(desktopMountBox).not.toBeNull();
  expect(filesystemBox).not.toBeNull();
  expect(desktopMountBox!.x + desktopMountBox!.width).toBeLessThanOrEqual(filesystemBox!.x);
  expect(await mount.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
    ),
  ).toBe(true);

  await page.setViewportSize({ width: 560, height: 900 });
  await expect(mount).toHaveCSS('overflow', 'visible');
  await expect(mount).toHaveCSS('text-overflow', 'clip');
  await expect(mount).toHaveCSS('white-space', 'normal');
  expect(await mount.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
    ),
  ).toBe(true);
});

test('logs tab streams entries', async ({ page }) => {
  await page.goto('/nodes/node-a');
  await waitForAppShell(page);
  await page.locator('[data-test="tab-logs"]').click();
  await expect(page.locator('[data-test="log-panel"]')).toBeVisible();
  await expect(page.locator('[data-test="log-entry"]')).toContainText('collector started');
});
