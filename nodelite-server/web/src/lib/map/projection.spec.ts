import { describe, expect, it } from 'vitest';
import { makeNode } from '@/api/__fixtures__/nodes';
import {
  MAP_HEIGHT,
  MAP_WIDTH,
  clamp01,
  hashString,
  mapProject,
  nodeFlag,
  nodePosition,
  nodeRegionKey,
  nodeStatusKey,
} from './projection';

describe('hashString', () => {
  it('is deterministic and non-negative', () => {
    expect(hashString('node-a')).toBe(hashString('node-a'));
    expect(hashString('node-a')).toBeGreaterThanOrEqual(0);
  });

  it('differs for different inputs', () => {
    expect(hashString('node-a')).not.toBe(hashString('node-b'));
  });
});

describe('clamp01', () => {
  it('clamps to [0.02, 0.98]', () => {
    expect(clamp01(-1)).toBe(0.02);
    expect(clamp01(5)).toBe(0.98);
    expect(clamp01(0.5)).toBe(0.5);
  });
});

describe('nodeRegionKey', () => {
  it('matches an explicit region tag', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: ['jp'] },
    });
    expect(nodeRegionKey(node)).toBe('jp');
  });

  it('matches a country:xx style tag', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: ['country:de'] },
    });
    expect(nodeRegionKey(node)).toBe('de');
  });

  it('matches a flag:xx style tag when geoip is unavailable', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'web-us-1', tags: ['flag:jp'] },
    });
    expect(nodeRegionKey(node)).toBe('jp');
  });

  it('does not infer a region from hostname tokens alone', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'web-us-1', tags: [] },
    });
    expect(nodeRegionKey(node)).toBeNull();
  });

  it('does not infer a region from node ids alone', () => {
    const node = makeNode({
      identity: { node_id: 'jp-edge-1', node_label: 'X', hostname: 'host', tags: [] },
    });
    expect(nodeRegionKey(node)).toBeNull();
  });

  it('uses geoip country even when hostname suggests a different region', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'web-in-1', tags: [] },
      geoip_country: 'US',
    });
    expect(nodeRegionKey(node)).toBe('us');
  });

  it('normalizes geoip country names to country codes', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: [] },
      geoip_country: 'United States',
    });
    expect(nodeRegionKey(node)).toBe('us');
  });

  it('keeps explicit tags ahead of geoip country', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: ['country:de'] },
      geoip_country: 'US',
    });
    expect(nodeRegionKey(node)).toBe('de');
  });

  it('keeps manual country ahead of explicit tags', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: ['country:de'] },
      geoip_country: 'US',
      location_override_country: 'JP',
    });
    expect(nodeRegionKey(node)).toBe('jp');
  });

  it('matches manual Chinese location names without coordinates', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: [] },
      geoip_country: 'CN',
      location_override_country: '香港',
    });
    expect(nodeRegionKey(node)).toBe('hk');
  });

  it('matches manual city aliases without coordinates', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: [] },
      geoip_country: 'US',
      location_override_country: 'JP',
      location_override_city: 'Osaka',
    });
    expect(nodeRegionKey(node)).toBe('jp');
  });

  it('normalizes manual country names before resolving flags', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: [] },
      location_override_country: 'Germany',
    });
    expect(nodeRegionKey(node)).toBe('de');
    expect(nodeFlag(node)).toBe('🇩🇪');
  });

  it('does not fall back to geoip country when a manual location is unknown', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: [] },
      geoip_country: 'US',
      location_override_country: 'Mars',
    });
    expect(nodeRegionKey(node)).toBeNull();
  });

  it('treats LAN geoip as local instead of falling back to hostname', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'web-in-1', tags: [] },
      geoip_country: 'LAN',
    });
    expect(nodeRegionKey(node)).toBe('lan');
  });

  it('returns null when nothing matches', () => {
    const node = makeNode({
      identity: { node_id: 'zzz', node_label: 'Z', hostname: 'host', tags: [] },
    });
    expect(nodeRegionKey(node)).toBeNull();
  });
});

describe('nodePosition', () => {
  it('is deterministic for the same node id', () => {
    const node = makeNode({
      identity: { node_id: 'srv-1', node_label: 'S', hostname: 'h', tags: ['jp'] },
    });
    expect(nodePosition(node)).toEqual(nodePosition(node));
  });

  it('places region-tagged nodes near the region anchor', () => {
    const node = makeNode({
      identity: { node_id: 'srv-1', node_label: 'S', hostname: 'h', tags: ['jp'] },
    });
    const { x, y } = nodePosition(node);
    // jp anchor is [0.86, 0.42]; jitter is < ±0.02
    expect(x).toBeGreaterThan(0.82);
    expect(x).toBeLessThan(0.9);
    expect(y).toBeGreaterThan(0.38);
    expect(y).toBeLessThan(0.46);
  });

  it('uses explicit coordinates before region anchors', () => {
    const node = makeNode({
      identity: { node_id: 'hk-01', node_label: 'HK', hostname: 'h', tags: ['country:us'] },
      location_override_country: 'HK',
      location_override_city: 'Hong Kong',
      location_override_latitude: 22.3193,
      location_override_longitude: 114.1694,
    });
    const projected = mapProject(114.1694, 22.3193);
    const { x, y } = nodePosition(node);
    expect(x).toBeCloseTo(projected.x / MAP_WIDTH, 5);
    expect(y).toBeCloseTo(projected.y / MAP_HEIGHT, 5);
  });

  it('does not reuse geoip coordinates for a manual text-only location', () => {
    const node = makeNode({
      identity: { node_id: 'hk-02', node_label: 'HK', hostname: 'h', tags: [] },
      geoip_country: 'CN',
      geoip_city: 'Shenyang',
      geoip_latitude: 41.8057,
      geoip_longitude: 123.4315,
      location_override_country: '香港',
    });
    const { x, y } = nodePosition(node);
    expect(x).toBeGreaterThan(0.75);
    expect(x).toBeLessThan(0.83);
    expect(y).toBeGreaterThan(0.46);
    expect(y).toBeLessThan(0.54);
  });

  it('keeps unknown-region nodes within the safe band', () => {
    const node = makeNode({
      identity: { node_id: 'mystery', node_label: 'M', hostname: 'h', tags: [] },
    });
    const { x, y } = nodePosition(node);
    expect(x).toBeGreaterThanOrEqual(0.15);
    expect(x).toBeLessThanOrEqual(0.85);
    expect(y).toBeGreaterThanOrEqual(0.2);
    expect(y).toBeLessThanOrEqual(0.8);
  });

  it('pins LAN nodes near the local-network corner', () => {
    const node = makeNode({
      identity: { node_id: 'lan-1', node_label: 'L', hostname: 'h', tags: [] },
      geoip_country: 'LAN',
    });
    const { x, y } = nodePosition(node);
    expect(x).toBeGreaterThanOrEqual(0.02);
    expect(x).toBeLessThanOrEqual(0.14);
    expect(y).toBeGreaterThanOrEqual(0.02);
    expect(y).toBeLessThanOrEqual(0.18);
  });
});

describe('nodeStatusKey', () => {
  it('returns offline when not online', () => {
    expect(nodeStatusKey(makeNode({ online: false }))).toBe('offline');
  });

  it('returns latency when online but over the warn threshold', () => {
    expect(nodeStatusKey(makeNode({ online: true, latency_ms: 250 }))).toBe('latency');
  });

  it('returns online when healthy', () => {
    expect(nodeStatusKey(makeNode({ online: true, latency_ms: 20 }))).toBe('online');
  });

  it('returns online when latency is null', () => {
    expect(nodeStatusKey(makeNode({ online: true, latency_ms: null }))).toBe('online');
  });
});

describe('nodeFlag', () => {
  it('returns the region flag when known', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'h', tags: ['us'] },
    });
    expect(nodeFlag(node)).toBe('🇺🇸');
  });

  it('falls back to a globe for unknown regions', () => {
    const node = makeNode({
      identity: { node_id: 'zzz', node_label: 'Z', hostname: 'host', tags: [] },
    });
    expect(nodeFlag(node)).toBe('🌐');
  });

  it('returns a local flag for LAN geoip nodes', () => {
    const node = makeNode({
      identity: { node_id: 'x', node_label: 'X', hostname: 'web-in-1', tags: [] },
      geoip_country: 'LAN',
    });
    expect(nodeFlag(node)).toBe('🏠');
  });
});

describe('mapProject', () => {
  it('maps lon=0 to the horizontal centre', () => {
    expect(mapProject(0, 0).x).toBeCloseTo(MAP_WIDTH / 2, 5);
  });

  it('maps lon=-180 to the left edge and lon=180 to the right edge', () => {
    expect(mapProject(-180, 0).x).toBeCloseTo(0, 5);
    expect(mapProject(180, 0).x).toBeCloseTo(MAP_WIDTH, 5);
  });

  it('places higher latitudes higher on the canvas (smaller y)', () => {
    expect(mapProject(0, 60).y).toBeLessThan(mapProject(0, -20).y);
  });

  it('keeps the equator near vertical mid + shift, within bounds', () => {
    const y = mapProject(0, 0).y;
    expect(y).toBeGreaterThan(0);
    expect(y).toBeLessThan(MAP_HEIGHT);
  });
});
