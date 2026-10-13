import { expect, test } from '@playwright/test';
import { setupApiFixtures, waitForAppShell } from './_helpers';

// Plan §3.7.2 flow 4: theme toggle (dark ↔ light).
// Validation points:
//   - Toggling changes the active theme without a visible flash.
//   - Choice persists in localStorage and survives reload.
test('theme toggle persists across reload', async ({ page }) => {
  await setupApiFixtures(page);
  await page.goto('/');
  await waitForAppShell(page);

  const initial = await page.locator('html').getAttribute('data-theme');
  await page.locator('[data-test="theme-toggle"]').click();

  const next = initial === 'dark' ? 'light' : 'dark';
  await expect(page.locator('html')).toHaveAttribute('data-theme', next);

  await page.reload();
  await waitForAppShell(page);
  await expect(page.locator('html')).toHaveAttribute('data-theme', next);
});

test('appearance persists across routes and reload independently of color mode @mobile', async ({
  page,
}) => {
  await setupApiFixtures(page);
  await page.route('**/api/audit-log?*', (route) => route.fulfill({ json: [] }));
  await page.goto('/');
  await waitForAppShell(page);
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-appearance', 'classic');
  await page.locator('[data-test="node-list"]').scrollIntoViewIfNeeded();
  const originalRadius = await page
    .locator('[data-test="node-card"]')
    .first()
    .evaluate((element) => getComputedStyle(element).borderRadius);

  await page.locator('[data-test="appearance-select"]').selectOption('swiss');
  await expect(html).toHaveAttribute('data-appearance', 'swiss');
  await page.locator('[data-test="theme-toggle"]').click();
  const colorMode = await html.getAttribute('data-theme');

  for (const language of ['en', 'zh-CN']) {
    await page.locator('[data-test="language-select"]').selectOption(language);
    for (const path of ['/settings', '/account', '/alerts', '/logs', '/nodes/node-a']) {
      await page.goto(path);
      await waitForAppShell(page);
      await expect(html).toHaveAttribute('data-appearance', 'swiss');
      await expect(html).toHaveAttribute('data-theme', colorMode!);
      const layout = await page.evaluate(
        (viewportWidth) => ({
          width: document.documentElement.scrollWidth,
          overflow: [...document.querySelectorAll('body *')]
            .filter((element) => element.getBoundingClientRect().right > viewportWidth + 1)
            .slice(0, 8)
            .map((element) => ({
              class: element.className,
              right: element.getBoundingClientRect().right,
            })),
        }),
        page.viewportSize()!.width,
      );
      expect(
        layout.width,
        `${language} ${path}: ${JSON.stringify(layout.overflow)}`,
      ).toBeLessThanOrEqual(page.viewportSize()!.width);
    }
  }
  await page.reload();
  await waitForAppShell(page);
  await expect(html).toHaveAttribute('data-appearance', 'swiss');
  await expect(html).toHaveAttribute('data-theme', colorMode!);
  await page.goto('/');
  await waitForAppShell(page);
  await page.locator('[data-test="node-list"]').scrollIntoViewIfNeeded();
  const card = page.locator('[data-test="node-card"]').first();
  await expect(card).toHaveCSS('border-radius', '0px');
  await page.locator('[data-test="appearance-select"]').selectOption('classic');
  await expect(card).toHaveCSS('border-radius', originalRadius);
  await expect(html).toHaveAttribute('data-theme', colorMode!);
});

test('Swiss primary buttons have readable contrast in both color modes @mobile', async ({
  page,
}) => {
  await setupApiFixtures(page);
  await page.goto('/settings');
  await waitForAppShell(page);
  await page.locator('[data-test="appearance-select"]').selectOption('swiss');
  await page.locator('[data-test="open-install-agent"]').waitFor();

  for (let mode = 0; mode < 2; mode++) {
    const contrasts = await page.locator('.btn--primary').evaluateAll((buttons) => {
      const luminance = (rgb: string): number => {
        const channels = (rgb.match(/[\d.]+/g) ?? [])
          .slice(0, 3)
          .map(Number)
          .map((value) => {
            const channel = value / 255;
            return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
          });
        return channels[0]! * 0.2126 + channels[1]! * 0.7152 + channels[2]! * 0.0722;
      };
      return buttons.map((button) => {
        const style = getComputedStyle(button);
        const foreground = luminance(style.color);
        const background = luminance(style.backgroundColor);
        return (
          (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05)
        );
      });
    });
    expect(contrasts.length).toBeGreaterThan(0);
    for (const contrast of contrasts) expect(contrast).toBeGreaterThanOrEqual(4.5);
    await page.locator('[data-test="theme-toggle"]').click();
  }
});
