import { randomBytes } from 'node:crypto';
import { expect, test as base, type APIRequestContext, type Page } from '@playwright/test';

interface LiveAgent {
  id: string;
  label: string;
  update(cpu: number): Promise<void>;
  remove(): Promise<void>;
}

function snapshot(cpu: number) {
  return {
    collected_at: new Date().toISOString(),
    cpu_usage_percent: cpu,
    load: { one: 0.42, five: 0.5, fifteen: 0.6 },
    memory: {
      total_bytes: 8_000_000_000,
      used_bytes: 2_400_000_000,
      available_bytes: 5_600_000_000,
      swap_total_bytes: 0,
      swap_used_bytes: 0,
    },
    uptime_secs: 90_000,
    disks: [],
    network: {
      total_rx_bytes: 1000,
      total_tx_bytes: 2000,
      rx_bytes_per_sec: 10,
      tx_bytes_per_sec: 20,
      packet_loss_percent: 0,
    },
  };
}

async function issueAgent(request: APIRequestContext, id: string, label: string) {
  const response = await request.post('/api/settings/agents/install', {
    data: { node_id: id, node_label: label, current_password: process.env.NODELITE_E2E_PASS },
  });
  expect(response.status()).toBe(200);
  const { install_command: command } = await response.json();
  const installToken = /NODELITE_AGENT_INSTALL_TOKEN='([a-f0-9]+)'/.exec(command)?.[1];
  if (!installToken) throw new Error('Install response did not contain a one-time test token');
  // The install endpoint uses Bearer auth, independently of the browser's Basic credentials.
  const bootstrap = await fetch(new URL('/install/bootstrap', response.url()), {
    headers: { Authorization: `Bearer ${installToken}` },
    signal: AbortSignal.timeout(10_000),
  });
  expect(bootstrap.status).toBe(200);
  const token = /^token = "([a-f0-9]+)"$/m.exec(await bootstrap.text())?.[1];
  if (!token) throw new Error('Bootstrap did not return a test agent token');
  return token;
}

function authenticate(socket: WebSocket, id: string, label: string, token: string) {
  return new Promise<void>((resolve, reject) => {
    const deadline = setTimeout(
      () => reject(new Error('Test Agent authentication timed out')),
      10_000,
    );
    const fail = () => {
      clearTimeout(deadline);
      reject(new Error('Test Agent connection failed'));
    };
    socket.addEventListener('error', fail, { once: true });
    socket.addEventListener('close', fail, { once: true });
    socket.addEventListener(
      'open',
      () => {
        socket.send(
          JSON.stringify({
            type: 'hello',
            protocol_version: 3,
            token,
            identity: {
              node_id: id,
              node_label: label,
              hostname: 'live-e2e-host',
              os: 'linux',
              kernel_version: '6.1.0',
              cpu_model: 'E2E CPU',
              cpu_cores: 2,
              agent_version: '0.1.0',
              boot_time: null,
              tags: [],
            },
          }),
        );
      },
      { once: true },
    );
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(String(event.data));
      if (message.type === 'ping')
        socket.send(JSON.stringify({ type: 'pong', nonce: message.nonce }));
      if (message.type === 'server_notice') {
        clearTimeout(deadline);
        if (message.message === 'authenticated') resolve();
        else reject(new Error('Test Agent authentication rejected'));
      }
    });
  });
}

export const test = base.extend<{ agent: LiveAgent }>({
  agent: [
    async ({ request, baseURL }, use) => {
      const id = `live-${randomBytes(8).toString('hex')}`;
      const label = `Live node ${id}`;
      const token = await issueAgent(request, id, label);
      const socket = new WebSocket(`${baseURL!.replace(/^http/, 'ws')}/ws`);
      let cpu = 12;
      let timer: ReturnType<typeof setInterval> | undefined;
      let removed = false;
      const send = () => {
        if (socket.readyState === WebSocket.OPEN)
          socket.send(JSON.stringify({ type: 'metrics', snapshot: snapshot(cpu) }));
      };
      const agent: LiveAgent = {
        id,
        label,
        async update(value) {
          cpu = value;
          send();
          await expect
            .poll(async () => {
              const response = await request.get(`/api/nodes/${id}`);
              return (await response.json()).snapshot?.cpu_usage_percent;
            })
            .toBe(value);
        },
        async remove() {
          if (removed) return;
          const response = await request.delete(`/api/settings/agents/${id}`, {
            data: { current_password: process.env.NODELITE_E2E_PASS },
          });
          expect(response.status()).toBe(200);
          removed = true;
        },
      };
      try {
        await authenticate(socket, id, label, token);
        await agent.update(cpu);
        timer = setInterval(send, 1000);
        await use(agent);
      } finally {
        clearInterval(timer);
        socket.close();
        await agent.remove();
      }
    },
    { auto: true },
  ],
});

export { expect };

export async function setVisible(page: Page, visible: boolean) {
  await page.evaluate((hidden) => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
    document.dispatchEvent(new Event('visibilitychange'));
  }, !visible);
}

export function nodeCard(page: Page, id: string) {
  return page.locator(`[data-test="node-card"][data-node-id="${id}"]`);
}
