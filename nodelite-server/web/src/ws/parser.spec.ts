import { describe, expect, it } from 'vitest';
import { makeNode, makeOverview } from '@/api/__fixtures__/nodes';
import { parseBrowserMessage } from './parser';

describe('browser protocol parser', () => {
  it('accepts the complete server shapes and nullable offline snapshot', () => {
    const node = makeNode();
    for (const message of [
      { type: 'initial_state', generated_at: 'now', overview: makeOverview(), nodes: [node] },
      { type: 'overview_update', generated_at: 'now', overview: makeOverview() },
      { type: 'node_upsert', generated_at: 'now', node: { ...node, snapshot: null } },
      { type: 'node_removed', generated_at: 'now', node_id: 'example' },
      { type: 'ping' }, { type: 'pong' },
    ]) expect(parseBrowserMessage(message)).toEqual(message);
  });

  it('rejects invalid array members and missing or invalid nested fields', () => {
    for (const node of [null, {}, { ...makeNode(), identity: null },
      { ...makeNode(), snapshot: undefined },
      { ...makeNode(), snapshot: { ...makeNode().snapshot, memory: {} } },
      { ...makeNode(), identity: { ...makeNode().identity, tags: [1] } },
      { ...makeNode(), latitude_ms: 0, latency_ms: NaN },
      { ...makeNode(), geoip_latitude: 100 }]) {
      expect(parseBrowserMessage({ type: 'node_upsert', generated_at: 'now', node })).toBeNull();
      expect(parseBrowserMessage({ type: 'initial_state', generated_at: 'now', overview: makeOverview(), nodes: [node] })).toBeNull();
    }
    expect(parseBrowserMessage({ type: 'overview_update', generated_at: 'now', overview: {} })).toBeNull();
    expect(parseBrowserMessage({ type: 'overview_update', generated_at: 'now', overview: makeOverview({ online_nodes: Infinity }) })).toBeNull();
  });
});
