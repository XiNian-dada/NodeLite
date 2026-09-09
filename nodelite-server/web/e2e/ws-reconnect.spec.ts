import type { WebSocketRoute } from '@playwright/test';
import { test, expect, nodeCard } from './_live';

test('WS drop triggers reconnect and recovers without a visibility change', async ({
  page,
  agent,
}) => {
  let blocked = false;
  let attempts = 0;
  let active: WebSocketRoute | undefined;
  await page.routeWebSocket('**/ws/browser', (socket) => {
    attempts++;
    if (blocked) socket.close();
    else {
      active = socket;
      socket.connectToServer();
    }
  });
  await page.goto('/');
  const cpu = nodeCard(page, agent.id).locator('[data-test="metric-cpu"]');
  await expect(cpu).toHaveText('12%');
  blocked = true;
  active!.close();
  await expect(page.locator('[data-test="connection-status"]')).toBeVisible();
  await expect.poll(() => attempts).toBeGreaterThan(1);
  await agent.update(61);
  await expect(cpu).toHaveText('61%');
  blocked = false;
  await expect(page.locator('[data-test="connection-status"]')).toBeHidden();
  await agent.update(72);
  await expect(cpu).toHaveText('72%');
});
