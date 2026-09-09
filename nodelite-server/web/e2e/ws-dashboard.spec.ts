import type { WebSocketRoute } from '@playwright/test';
import { test, expect, nodeCard, setVisible } from './_live';

test.describe('WebSocket Dashboard', () => {
  test('loads dashboard via WS InitialState without REST polling', async ({ page, agent }) => {
    const restCalls: string[] = [];
    let initialStates = 0;
    page.on('request', (request) => {
      if (/\/api\/(overview|nodes)$/.test(request.url())) restCalls.push(request.url());
    });
    page.on('websocket', (socket) =>
      socket.on('framereceived', ({ payload }) => {
        if (JSON.parse(String(payload)).type === 'initial_state') initialStates++;
      }),
    );
    await page.goto('/');
    await expect(nodeCard(page, agent.id).locator('[data-test="metric-cpu"]')).toHaveText('12%');
    await expect.poll(() => initialStates).toBe(1);
    await expect(page.locator('[data-test="connection-status"]')).toBeHidden();
    // Cover the fallback deadline, not just the first render.
    await page.waitForTimeout(1000);
    expect(restCalls).toHaveLength(0);
  });

  test('incremental node updates arrive via WebSocket and removal deletes the card', async ({
    page,
    agent,
  }) => {
    const restCalls: string[] = [];
    const events: { type: string; node_id?: string; node?: { identity: { node_id: string } } }[] =
      [];
    page.on('request', (request) => {
      if (/\/api\/(overview|nodes)$/.test(request.url())) restCalls.push(request.url());
    });
    page.on('websocket', (socket) =>
      socket.on('framereceived', ({ payload }) => {
        events.push(JSON.parse(String(payload)));
      }),
    );
    await page.goto('/');
    const card = nodeCard(page, agent.id);
    const cpu = card.locator('[data-test="metric-cpu"]');
    await expect(cpu).toHaveText('12%');
    const documentTime = await page.evaluate(() => performance.timeOrigin);

    await agent.update(73);
    await expect(cpu).toHaveText('73%');
    expect(
      events.some(
        (event) => event.type === 'node_upsert' && event.node?.identity.node_id === agent.id,
      ),
    ).toBe(true);

    await agent.remove();
    await expect(card).toHaveCount(0);
    expect(
      events.some((event) => event.type === 'node_removed' && event.node_id === agent.id),
    ).toBe(true);
    expect(await page.evaluate(() => performance.timeOrigin)).toBe(documentTime);
    expect(restCalls).toHaveLength(0);
  });

  test('falls back to REST when WebSocket is blocked', async ({ page, agent }) => {
    await page.routeWebSocket('**/ws/browser', (socket) => socket.close());
    await page.goto('/');
    const cpu = nodeCard(page, agent.id).locator('[data-test="metric-cpu"]');
    await expect(cpu).toHaveText('12%');
    await expect(page.locator('[data-test="connection-status"]')).toBeVisible();
    await agent.update(23);
    await expect(cpu).toHaveText('23%');
    await agent.update(34);
    await expect(cpu).toHaveText('34%');
  });

  test('WebSocket connection persists across route navigation', async ({ page, agent }) => {
    let connections = 0;
    page.on('websocket', () => connections++);
    await page.goto('/');
    await expect(nodeCard(page, agent.id)).toBeVisible();
    const documentTime = await page.evaluate(() => performance.timeOrigin);
    for (const [index, route] of ['settings', 'account', 'alerts', 'logs'].entries()) {
      await page.locator(`[data-test="nav-${route}"]`).click();
      await expect(page).toHaveURL(new RegExp(`/${route}$`));
      await agent.update(40 + index);
      await page.locator('[data-test="nav-overview"]').click();
      await expect(nodeCard(page, agent.id).locator('[data-test="metric-cpu"]')).toHaveText(
        `${40 + index}%`,
      );
    }
    expect(connections).toBe(1);
    expect(await page.evaluate(() => performance.timeOrigin)).toBe(documentTime);
  });

  test('closes WebSocket when tab hidden, reconnects when visible', async ({ page, agent }) => {
    let connections = 0;
    let closed = 0;
    page.on('websocket', (socket) => {
      connections++;
      socket.on('close', () => closed++);
    });
    await page.goto('/');
    await expect(nodeCard(page, agent.id)).toBeVisible();
    await setVisible(page, false);
    await expect.poll(() => closed).toBe(1);
    await setVisible(page, true);
    await expect.poll(() => connections).toBe(2);
    await expect(page.locator('[data-test="connection-status"]')).toBeHidden();
    await expect(nodeCard(page, agent.id)).toBeVisible();
  });

  test('displays reconnecting state when connection drops', async ({ page, agent }) => {
    let blocked = false;
    let active: WebSocketRoute | undefined;
    await page.routeWebSocket('**/ws/browser', (socket) => {
      if (blocked) socket.close();
      else {
        active = socket;
        socket.connectToServer();
      }
    });
    await page.goto('/');
    await expect(nodeCard(page, agent.id)).toBeVisible();
    blocked = true;
    active!.close();
    await expect(page.locator('[data-test="connection-status"]')).toBeVisible();
    await expect(nodeCard(page, agent.id)).toBeVisible();
  });
});
