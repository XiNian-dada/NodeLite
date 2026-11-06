import type { BrowserMessage } from '@/api/types';

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function numeric(value: unknown): boolean {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

function nullableNumber(value: unknown): boolean {
  return value === null || numeric(value);
}

function nullableString(value: unknown): boolean {
  return value === null || typeof value === 'string';
}

function overview(value: unknown): boolean {
  return record(value) && typeof value.generated_at === 'string' &&
    ['total_nodes', 'online_nodes', 'offline_nodes', 'total_rx_bytes', 'total_tx_bytes',
      'current_rx_bytes_per_sec', 'current_tx_bytes_per_sec'].every((key) => numeric(value[key])) &&
    nullableNumber(value.average_latency_ms);
}

function snapshot(value: unknown): boolean {
  return value === null || (record(value) && nullableNumber(value.cpu_usage_percent) &&
    record(value.load) && numeric(value.load.one) && record(value.memory) &&
    numeric(value.memory.total_bytes) && numeric(value.memory.used_bytes));
}

function coordinate(value: unknown, limit: number): boolean {
  return value === null || (typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= limit);
}

function node(value: unknown): boolean {
  if (!record(value) || !record(value.identity)) return false;
  const identity = value.identity;
  return ['node_id', 'node_label', 'hostname'].every((key) => typeof identity[key] === 'string') &&
    Array.isArray(identity.tags) && identity.tags.every((tag) => typeof tag === 'string') &&
    ['geoip_country', 'geoip_city', 'location_override_country', 'location_override_city']
      .every((key) => nullableString(value[key])) &&
    ['geoip_latitude', 'location_override_latitude'].every((key) => coordinate(value[key], 90)) &&
    ['geoip_longitude', 'location_override_longitude'].every((key) => coordinate(value[key], 180)) &&
    snapshot(value.snapshot) && nullableNumber(value.latency_ms) && typeof value.online === 'boolean';
}

// Unknown extra fields remain compatible with newer servers, but every consumed field is checked.
export function parseBrowserMessage(value: unknown): BrowserMessage | null {
  if (!record(value) || typeof value.type !== 'string') return null;
  if (value.type === 'ping' || value.type === 'pong') return value as BrowserMessage;
  if (typeof value.generated_at !== 'string') return null;
  switch (value.type) {
    case 'initial_state':
      return overview(value.overview) && Array.isArray(value.nodes) && value.nodes.every(node)
        ? value as BrowserMessage : null;
    case 'overview_update':
      return overview(value.overview) ? value as BrowserMessage : null;
    case 'node_upsert':
      return node(value.node) ? value as BrowserMessage : null;
    case 'node_removed':
      return typeof value.node_id === 'string' ? value as BrowserMessage : null;
    default:
      return null;
  }
}
