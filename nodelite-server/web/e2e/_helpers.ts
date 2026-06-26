import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import type { Page, Route } from '@playwright/test';

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

const GENERATED_AT = '2026-05-29T00:00:00Z';

const node = {
  identity: {
    node_id: 'node-a',
    node_label: 'Node A',
    hostname: 'host-a',
    tags: ['edge'],
  },
  geoip_country: 'JP',
  geoip_city: 'Tokyo',
  geoip_latitude: 35.6762,
  geoip_longitude: 139.6503,
  location_override_country: null,
  location_override_city: null,
  location_override_latitude: null,
  location_override_longitude: null,
  snapshot: {
    cpu_usage_percent: 35,
    load: { one: 0.42 },
    memory: { total_bytes: 8_000_000_000, used_bytes: 2_400_000_000 },
  },
  latency_ms: 12,
  online: true,
};

const nodeStatus = {
  ...node,
  identity: {
    ...node.identity,
    os: 'linux',
    kernel_version: '6.1.0',
    cpu_model: 'Test CPU',
    cpu_cores: 4,
    agent_version: '1.0.0',
    boot_time: '2026-05-28T00:00:00Z',
  },
  remote_ip: '203.0.113.7',
  snapshot: {
    collected_at: GENERATED_AT,
    cpu_usage_percent: 35,
    load: { one: 0.42, five: 0.5, fifteen: 0.6 },
    memory: {
      total_bytes: 8_000_000_000,
      used_bytes: 2_400_000_000,
      available_bytes: 5_600_000_000,
      swap_total_bytes: 0,
      swap_used_bytes: 0,
    },
    uptime_secs: 90_000,
    disks: [
      {
        device: '/dev/sda1',
        mount_point: '/',
        fs_type: 'ext4',
        total_bytes: 100_000_000_000,
        available_bytes: 60_000_000_000,
        used_bytes: 40_000_000_000,
        used_percent: 40,
      },
    ],
    network: {
      total_rx_bytes: 1000,
      total_tx_bytes: 2000,
      rx_bytes_per_sec: 10,
      tx_bytes_per_sec: 20,
      packet_loss_percent: 0.2,
    },
  },
  last_seen: GENERATED_AT,
};

const history = Array.from({ length: 8 }, (_, index) => ({
  node_id: 'node-a',
  recorded_at: new Date(Date.UTC(2026, 4, 29, 0, index)).toISOString(),
  cpu_usage_percent: 25 + index,
  load_one: 0.3 + index / 10,
  load_five: 0.4 + index / 10,
  load_fifteen: 0.5 + index / 10,
  memory_used_percent: 30 + index,
  rx_bytes_per_sec: 100 + index,
  tx_bytes_per_sec: 80 + index,
  latency_ms: 10 + index,
  packet_loss_percent: 0,
  disk_used_percent: 40 + index,
}));

const settings = {
  service: 'nodelite-server',
  server_version: '2.3.0',
  repository: 'https://github.com/XiNian-dada/NodeLite',
  public_base_url: 'http://localhost:8080',
  listen: '127.0.0.1:8080',
  config_path: '/etc/nodelite/server.toml',
  registry_path: '/var/lib/nodelite/registry.json',
  history_db_path: '/var/lib/nodelite/history.db',
  snapshot_path: '/var/lib/nodelite/snapshot.json',
  history_retention_hours: 336,
  refresh_interval_secs: 5,
  auth: {
    enabled: true,
    username: 'admin',
    two_factor_enabled: false,
    totp_secret_configured: false,
    session_ttl_secs: 86_400,
    pending_ttl_secs: 300,
  },
  updates: {
    latest_release_url: 'https://github.com/XiNian-dada/NodeLite/releases/latest',
    server_upgrade_command: 'curl -fsSL https://example/install.sh | sh',
    agent_upgrade_command: 'curl -fsSL https://example/agent.sh | sh',
  },
  agents: [
    {
      node_id: 'node-a',
      node_label: 'Node A',
      online: true,
      agent_version: '1.0.0',
      remote_ip: '203.0.113.7',
      tags: ['edge'],
      token_expires_at: '2026-12-01T00:00:00Z',
      token_expires_in_secs: 1_000_000,
      service_expires_at: null,
      service_unlimited: false,
      renewal_price: null,
      geoip_country: 'JP',
      geoip_city: 'Tokyo',
      geoip_latitude: 35.6762,
      geoip_longitude: 139.6503,
      location_override_country: null,
      location_override_city: null,
      location_override_latitude: null,
      location_override_longitude: null,
    },
  ],
};

const alertSettings = {
  config: {
    enabled: true,
    smtp: {
      enabled: false,
      host: 'smtp.example.com',
      port: 587,
      username: 'mailer',
      sender: 'alerts@example.com',
      recipients: ['ops@example.com'],
      transport: 'start_tls',
      send_resolved: true,
      password_configured: false,
    },
    webhook: {
      enabled: false,
      url: '',
      send_resolved: true,
      secret_configured: false,
    },
    rules: [
      {
        id: 'cpu-hot',
        name: 'CPU hot',
        enabled: true,
        metric: 'cpu_usage_percent',
        comparator: 'gt',
        threshold: 85,
        window_minutes: 5,
        severity: 'warning',
        scope_mode: 'all',
        node_ids: [],
        tags: [],
        delivery: ['smtp'],
        cooldown_minutes: 30,
        send_resolved: true,
      },
    ],
    inspection: {
      enabled: true,
      local_time: '09:00',
      lookback_hours: 24,
      delivery: ['smtp'],
      offline_grace_minutes: 10,
      latency_warn_ms: 250,
      cpu_warn_percent: 85,
      memory_warn_percent: 90,
    },
  },
  preview: {
    generated_at: GENERATED_AT,
    triggered_rules: [],
    inspection: {
      total_nodes: 1,
      offline_nodes: 0,
      latency_nodes: 0,
      cpu_hot_nodes: 0,
      memory_hot_nodes: 0,
      highlights: [],
    },
  },
};

function json(route: Route, body: unknown): Promise<void> {
  return route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });
}

export async function setupApiFixtures(page: Page): Promise<void> {
  const dictionary = await readFile(resolve('public/assets/ui-i18n.json'), 'utf8');

  await page.route('**/assets/ui-i18n.json', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: dictionary }),
  );
  await page.route('**/ws/browser', (route) => route.abort());
  await page.route('**/api/bootstrap', (route) =>
    json(route, {
      service: 'nodelite-server',
      status: 'ready',
      ready: true,
      history_available: true,
      public_base_url: 'http://localhost:8080',
      refresh_interval_secs: 5,
      registered_nodes: 1,
      geoip_enabled: true,
      geoip_provider: 'custom',
    }),
  );
  await page.route('**/api/settings', (route) => json(route, settings));
  await page.route('**/api/overview', (route) =>
    json(route, {
      generated_at: GENERATED_AT,
      total_nodes: 1,
      online_nodes: 1,
      offline_nodes: 0,
      total_rx_bytes: 1000,
      total_tx_bytes: 2000,
      current_rx_bytes_per_sec: 10,
      current_tx_bytes_per_sec: 20,
      average_latency_ms: 12,
    }),
  );
  await page.route('**/api/nodes', (route) => json(route, [node]));
  await page.route('**/api/nodes/node-a', (route) => json(route, nodeStatus));
  await page.route('**/api/nodes/node-a/history**', (route) => json(route, history));
  await page.route('**/api/nodes/node-a/logs**', (route) =>
    json(route, [{ occurred_at: GENERATED_AT, level: 'info', message: 'collector started' }]),
  );
  await page.route('**/api/settings/alerts', (route) => json(route, alertSettings));
  await page.route('**/api/settings/password', (route) =>
    json(route, { ok: true, message: 'Password changed' }),
  );
  await page.route('**/api/settings/2fa/start', (route) =>
    json(route, {
      secret: 'SECRET123',
      otpauth_uri: 'otpauth://totp/NodeLite:admin?secret=SECRET123',
      qr_svg: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"></svg>',
    }),
  );
  await page.route('**/api/settings/2fa/enable', (route) =>
    json(route, { ok: true, message: '2FA enabled' }),
  );
}

export async function setupVerify2faFixture(page: Page): Promise<void> {
  const html = await readFile(resolve('public/verify-2fa.html'), 'utf8');

  await page.route('**/verify-2fa', (route) =>
    route.fulfill({ status: 200, contentType: 'text/html', body: html }),
  );
  await page.route('**/api/verify-2fa', async (route) => {
    const body = route.request().postDataJSON() as { code?: string };
    if (body.code === '123456') {
      await json(route, { ok: true });
      return;
    }
    await route.fulfill({
      status: 429,
      contentType: 'application/json',
      body: JSON.stringify({ error: 'Rate limited' }),
    });
  });
}
