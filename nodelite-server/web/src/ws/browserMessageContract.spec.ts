import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WS } from 'vitest-websocket-mock';
import contract from './__fixtures__/browser_messages.json';
import { parseBrowserMessage, WsClient, type WsClientLogger } from './client';

const wsUrl = 'ws://localhost:1240/ws/browser-contract';
const serverMessageKeys = [
  'initial_state',
  'overview_update',
  'node_upsert',
  'node_removed',
  'pong',
] as const;

function sortedKeys(value: object): string[] {
  return Object.keys(value).sort();
}

describe('BrowserMessage contract fixture', () => {
  let server: WS | undefined;
  let client: WsClient | undefined;
  let logger: WsClientLogger;

  beforeEach(() => {
    vi.useRealTimers();
    logger = {
      log: vi.fn(),
      warn: vi.fn(),
      error: vi.fn(),
    };
    Object.defineProperty(document, 'hidden', {
      configurable: true,
      value: false,
    });
  });

  afterEach(() => {
    client?.destroy();
    server?.close();
    vi.useRealTimers();
  });

  it('accepts every Rust-generated message through the runtime parser', () => {
    for (const key of serverMessageKeys) {
      const message = contract.server_to_browser[key];
      expect(parseBrowserMessage(message), key).toEqual(message);
    }

    expect(parseBrowserMessage(contract.browser_to_server.ping)).toEqual(
      contract.browser_to_server.ping,
    );
  });

  it('locks message and nested payload field names', () => {
    expect(sortedKeys(contract.server_to_browser.initial_state)).toEqual([
      'generated_at',
      'nodes',
      'overview',
      'type',
    ]);
    expect(sortedKeys(contract.server_to_browser.overview_update)).toEqual([
      'generated_at',
      'overview',
      'type',
    ]);
    expect(sortedKeys(contract.server_to_browser.node_upsert)).toEqual([
      'generated_at',
      'node',
      'type',
    ]);
    expect(sortedKeys(contract.server_to_browser.node_removed)).toEqual([
      'generated_at',
      'node_id',
      'type',
    ]);
    expect(sortedKeys(contract.server_to_browser.pong)).toEqual(['type']);
    expect(sortedKeys(contract.browser_to_server.ping)).toEqual(['type']);

    const initial = parseBrowserMessage(contract.server_to_browser.initial_state);
    expect(initial?.type).toBe('initial_state');
    if (!initial || initial.type !== 'initial_state') throw new Error('invalid initial_state fixture');

    expect(sortedKeys(initial.overview)).toEqual([
      'average_latency_ms',
      'current_rx_bytes_per_sec',
      'current_tx_bytes_per_sec',
      'generated_at',
      'offline_nodes',
      'online_nodes',
      'total_nodes',
      'total_rx_bytes',
      'total_tx_bytes',
    ]);

    const node = initial.nodes[0];
    expect(node).toBeDefined();
    if (!node) throw new Error('initial_state fixture should contain one node');

    expect(sortedKeys(node)).toEqual([
      'geoip_city',
      'geoip_country',
      'geoip_latitude',
      'geoip_longitude',
      'identity',
      'latency_ms',
      'location_override_city',
      'location_override_country',
      'location_override_latitude',
      'location_override_longitude',
      'online',
      'snapshot',
    ]);
    expect(sortedKeys(node.identity)).toEqual(['hostname', 'node_id', 'node_label', 'tags']);
    expect(node.identity.node_id).toBe('contract-node-01');
    expect(node.snapshot).not.toBeNull();
    if (!node.snapshot) throw new Error('contract node should contain a snapshot');
    expect(sortedKeys(node.snapshot)).toEqual(['cpu_usage_percent', 'load', 'memory']);
    expect(sortedKeys(node.snapshot.load)).toEqual(['one']);
    expect(sortedKeys(node.snapshot.memory)).toEqual(['total_bytes', 'used_bytes']);

    const upsert = parseBrowserMessage(contract.server_to_browser.node_upsert);
    expect(upsert?.type).toBe('node_upsert');
    if (!upsert || upsert.type !== 'node_upsert') throw new Error('invalid node_upsert fixture');
    expect(upsert.node).toEqual(node);
  });

  it('dispatches Rust-generated server messages through WsClient', async () => {
    server = new WS(wsUrl);
    client = new WsClient(wsUrl, logger);
    const initialHandler = vi.fn();
    const overviewHandler = vi.fn();
    const upsertHandler = vi.fn();
    const removedHandler = vi.fn();
    client.on('initial_state', initialHandler);
    client.on('overview_update', overviewHandler);
    client.on('node_upsert', upsertHandler);
    client.on('node_removed', removedHandler);

    client.connect();
    await server.connected;
    server.send(JSON.stringify(contract.server_to_browser.initial_state));
    server.send(JSON.stringify(contract.server_to_browser.overview_update));
    server.send(JSON.stringify(contract.server_to_browser.node_upsert));
    server.send(JSON.stringify(contract.server_to_browser.node_removed));
    await new Promise((resolve) => setTimeout(resolve, 50));

    expect(initialHandler).toHaveBeenCalledWith(contract.server_to_browser.initial_state);
    expect(overviewHandler).toHaveBeenCalledWith(contract.server_to_browser.overview_update);
    expect(upsertHandler).toHaveBeenCalledWith(contract.server_to_browser.node_upsert);
    expect(removedHandler).toHaveBeenCalledWith(contract.server_to_browser.node_removed);
  });

  it('serializes the browser heartbeat exactly like the Rust Ping fixture', async () => {
    vi.useFakeTimers();
    server = new WS(wsUrl);
    client = new WsClient(wsUrl, logger);

    client.connect();
    await vi.advanceTimersByTimeAsync(100);
    await server.connected;
    await vi.advanceTimersByTimeAsync(30_000);

    expect(server.messages).toContain(JSON.stringify(contract.browser_to_server.ping));
  });
});
