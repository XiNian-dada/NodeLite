import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

for (const modal of [
  { route: '/settings', trigger: 'delete-agent', name: 'Delete Node A' },
  { route: '/nodes/node-a', trigger: 'zoom-cpu', name: 'CPU Usage' },
  { route: '/settings', trigger: 'view-update-log', name: 'Server Update Console' },
]) {
  test(`${modal.trigger} keeps keyboard focus and restores its trigger`, async ({ page }) => {
    await setupApiFixtures(page);
    await page.route('**/api/settings/update/server/log?*', (route) =>
      route.fulfill({
        json: { exists: false, text: '', offset: 0, next_offset: 0 },
      }),
    );
    await page.goto(modal.route);
    await waitForAppShell(page);
    const trigger = page.locator(`[data-test="${modal.trigger}"]`).first();
    await trigger.focus();
    await page.keyboard.press('Enter');
    const dialog = page.getByRole('dialog', { name: modal.name, exact: true });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAccessibleName(modal.name);
    await expect(dialog.getByRole('button', { name: 'Close', exact: true })).toBeFocused();
    for (const key of [
      'Tab',
      'Tab',
      'Tab',
      'Tab',
      'Shift+Tab',
      'Shift+Tab',
      'Shift+Tab',
      'Shift+Tab',
    ]) {
      await page.keyboard.press(key);
      await expect
        .poll(() => dialog.evaluate((element) => element.contains(document.activeElement)))
        .toBe(true);
    }
    await trigger.focus();
    await expect
      .poll(() => dialog.evaluate((element) => element.contains(document.activeElement)))
      .toBe(true);
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();

    await page.keyboard.press('Enter');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
  });
}
