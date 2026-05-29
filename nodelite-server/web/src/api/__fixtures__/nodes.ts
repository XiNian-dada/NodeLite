import type { BootstrapResponse, NodeListItem, OverviewData } from '@/api';

export function makeBootstrap(
  overrides: Partial<BootstrapResponse> = {},
): BootstrapResponse {
  return {
    service: 'nodelite-server',
    status: 'ready',
    ready: true,
    history_available: true,
    public_base_url: 'http://localhost:8080',
    refresh_interval_secs: 5,
    registered_nodes: 3,
    ...overrides,
  };
}

export function makeNode(overrides: Partial<NodeListItem> = {}): NodeListItem {
  return {
    identity: {
      node_id: 'node-a',
      node_label: 'Node A',
      hostname: 'host-a',
      tags: [],
      ...overrides.identity,
    },
    snapshot: overrides.snapshot ?? {
      cpu_usage_percent: 12.5,
      load: { one: 0.3 },
      memory: { total_bytes: 8_000_000_000, used_bytes: 2_000_000_000 },
    },
    latency_ms: overrides.latency_ms ?? 5,
    online: overrides.online ?? true,
  };
}

export function makeOverview(overrides: Partial<OverviewData> = {}): OverviewData {
  return {
    generated_at: '2026-05-29T00:00:00Z',
    total_nodes: 3,
    online_nodes: 2,
    offline_nodes: 1,
    total_rx_bytes: 1000,
    total_tx_bytes: 2000,
    current_rx_bytes_per_sec: 10,
    current_tx_bytes_per_sec: 20,
    average_latency_ms: 7.5,
    ...overrides,
  };
}
