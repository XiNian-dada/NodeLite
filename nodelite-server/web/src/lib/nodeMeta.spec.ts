import { describe, expect, it } from 'vitest';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import { ipFromNode, locationFromNode } from './nodeMeta';

describe('locationFromNode', () => {
  it('reads a loc/region/city tag', () => {
    const node = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, tags: ['region:eu-west'] },
    });
    expect(locationFromNode(node)).toBe('eu-west');
  });

  it('returns null when no location tag', () => {
    const node = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, tags: ['env:prod'] },
    });
    expect(locationFromNode(node)).toBeNull();
  });
});

describe('ipFromNode', () => {
  it('prefers an ip/addr tag', () => {
    const node = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, tags: ['ip:10.0.0.5'] },
      remote_ip: '203.0.113.7',
    });
    expect(ipFromNode(node)).toBe('10.0.0.5');
  });

  it('falls back to remote_ip', () => {
    const node = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, tags: [] },
      remote_ip: '203.0.113.7',
    });
    expect(ipFromNode(node)).toBe('203.0.113.7');
  });

  it('returns null when neither is present', () => {
    const node = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, tags: [] },
      remote_ip: null,
    });
    expect(ipFromNode(node)).toBeNull();
  });
});
