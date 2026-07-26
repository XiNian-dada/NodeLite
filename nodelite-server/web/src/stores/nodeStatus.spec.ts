import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiAbortError, ApiError } from '@/api/client';
import { apiClient } from '@/api';
import { makeNode, makeNodeStatus } from '@/api/__fixtures__/nodes';
import { useNodeStatusStore } from './nodeStatus';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, nodeStatus: vi.fn() },
  };
});

const mockStatus = vi.mocked(apiClient.nodeStatus);

describe('useNodeStatusStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockStatus.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('loads the node status for an id', async () => {
    const status = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a' },
    });
    mockStatus.mockResolvedValueOnce(status);
    const store = useNodeStatusStore();

    await store.load('a');
    expect(mockStatus).toHaveBeenCalledWith('a');
    expect(store.data).toEqual(status);
    expect(store.nodeId).toBe('a');
  });

  it('clears stale data when switching to a different node', async () => {
    mockStatus.mockResolvedValueOnce(makeNodeStatus());
    const store = useNodeStatusStore();
    await store.load('a');
    expect(store.data).not.toBeNull();

    // Switch to b: data should clear immediately, before the fetch resolves.
    let resolve: (v: ReturnType<typeof makeNodeStatus>) => void = () => {};
    mockStatus.mockReturnValueOnce(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const pending = store.load('b');
    expect(store.nodeId).toBe('b');
    expect(store.data).toBeNull();
    resolve(makeNodeStatus());
    await pending;
  });

  it('refresh re-fetches the current node', async () => {
    mockStatus.mockResolvedValue(makeNodeStatus());
    const store = useNodeStatusStore();
    await store.load('a');
    await store.refresh();
    expect(mockStatus).toHaveBeenCalledTimes(2);
    expect(mockStatus).toHaveBeenLastCalledWith('a');
  });

  it('refresh is a no-op when no node is active', async () => {
    const store = useNodeStatusStore();
    await store.refresh();
    expect(mockStatus).not.toHaveBeenCalled();
  });

  it('records non-abort errors', async () => {
    mockStatus.mockRejectedValueOnce(new ApiError(404, 'node not found'));
    const store = useNodeStatusStore();
    await store.load('missing');
    expect(store.error).toBeInstanceOf(ApiError);
    expect(store.data).toBeNull();
  });

  it('swallows ApiAbortError silently', async () => {
    mockStatus.mockRejectedValueOnce(new ApiAbortError('redirect'));
    const store = useNodeStatusStore();
    await store.load('a');
    expect(store.error).toBeNull();
  });

  it('fetches the new node when switched while a request is in flight', async () => {
    // a is still pending when we navigate to b. The id-aware guard must NOT
    // swallow b's fetch (the bug: a plain loading guard left data null until
    // the next poll).
    const statusA = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a' },
    });
    const statusB = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'b' },
    });
    let resolveA: (v: ReturnType<typeof makeNodeStatus>) => void = () => {};
    mockStatus
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveA = r;
        }),
      )
      .mockResolvedValueOnce(statusB);

    const store = useNodeStatusStore();
    const loadA = store.load('a'); // in flight
    const loadB = store.load('b'); // must still fetch b

    expect(mockStatus).toHaveBeenCalledTimes(2);
    expect(mockStatus).toHaveBeenLastCalledWith('b');

    await loadB;
    expect(store.nodeId).toBe('b');
    expect(store.data?.identity.node_id).toBe('b');

    // a's late response is discarded (we're on b now).
    resolveA(statusA);
    await loadA;
    expect(store.data?.identity.node_id).toBe('b');
  });

  it('ignores an old same-id response after switching away and back', async () => {
    const staleA = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a' },
      snapshot: { ...makeNodeStatus().snapshot!, cpu_usage_percent: 10 },
    });
    const freshA = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a' },
      snapshot: { ...makeNodeStatus().snapshot!, cpu_usage_percent: 80 },
    });
    let resolveStaleA: (v: ReturnType<typeof makeNodeStatus>) => void = () => {};
    let resolveB: (v: ReturnType<typeof makeNodeStatus>) => void = () => {};
    let resolveFreshA: (v: ReturnType<typeof makeNodeStatus>) => void = () => {};
    mockStatus
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveStaleA = r;
        }),
      )
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveB = r;
        }),
      )
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveFreshA = r;
        }),
      );

    const store = useNodeStatusStore();
    const firstA = store.load('a');
    const loadB = store.load('b');
    const latestA = store.load('a');

    resolveStaleA(staleA);
    await firstA;
    expect(store.data).toBeNull();
    expect(store.loading).toBe(true);

    resolveB(makeNodeStatus({ identity: { ...makeNodeStatus().identity, node_id: 'b' } }));
    await loadB;
    expect(store.data).toBeNull();
    expect(store.loading).toBe(true);

    resolveFreshA(freshA);
    await latestA;
    expect(store.data?.snapshot?.cpu_usage_percent).toBe(80);
    expect(store.loading).toBe(false);
  });

  it('dedups concurrent fetches for the same node', async () => {
    let resolve: (v: ReturnType<typeof makeNodeStatus>) => void = () => {};
    mockStatus.mockReturnValueOnce(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const store = useNodeStatusStore();

    const first = store.load('a');
    const second = store.load('a'); // same id, in flight; no second request
    expect(mockStatus).toHaveBeenCalledTimes(1);

    resolve(makeNodeStatus());
    await Promise.all([first, second]);
    expect(mockStatus).toHaveBeenCalledTimes(1);
  });

  it('merges realtime summaries while preserving full detail fields', async () => {
    const initial = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a', os: 'freebsd' },
    });
    mockStatus.mockResolvedValueOnce(initial);
    const store = useNodeStatusStore();
    await store.load('a');

    store.applyRealtimeSummary(
      makeNode({
        identity: {
          node_id: 'a',
          node_label: 'Realtime A',
          hostname: 'realtime-a',
          tags: ['edge'],
        },
        snapshot: {
          cpu_usage_percent: 88,
          load: { one: 4.2 },
          memory: { total_bytes: 16_000, used_bytes: 12_000 },
        },
        latency_ms: 320,
        online: false,
      }),
    );

    expect(store.data).toMatchObject({
      identity: {
        node_id: 'a',
        node_label: 'Realtime A',
        hostname: 'realtime-a',
        os: 'freebsd',
        tags: ['edge'],
      },
      snapshot: {
        collected_at: initial.snapshot?.collected_at,
        cpu_usage_percent: 88,
        load: { one: 4.2, five: 0.4, fifteen: 0.5 },
        memory: {
          total_bytes: 16_000,
          used_bytes: 12_000,
          available_bytes: 4_000,
        },
        disks: initial.snapshot?.disks,
        network: initial.snapshot?.network,
      },
      latency_ms: 320,
      online: false,
    });
  });

  it('ignores realtime summaries for a different active node', async () => {
    const initial = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a' },
    });
    mockStatus.mockResolvedValueOnce(initial);
    const store = useNodeStatusStore();
    await store.load('a');

    store.applyRealtimeSummary(
      makeNode({
        identity: { node_id: 'b', node_label: 'B', hostname: 'b', tags: [] },
        latency_ms: 999,
      }),
    );

    expect(store.data).toEqual(initial);
  });

  it('accepts a null realtime snapshot', async () => {
    mockStatus.mockResolvedValueOnce(
      makeNodeStatus({ identity: { ...makeNodeStatus().identity, node_id: 'a' } }),
    );
    const store = useNodeStatusStore();
    await store.load('a');

    store.applyRealtimeSummary(
      makeNode({
        identity: { node_id: 'a', node_label: 'A', hostname: 'a', tags: [] },
        snapshot: null,
      }),
    );

    expect(store.data?.snapshot).toBeNull();
  });

  it('marks the active node as removed and ignores its late REST response', async () => {
    let resolve: (value: ReturnType<typeof makeNodeStatus>) => void = () => {};
    mockStatus.mockReturnValueOnce(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const store = useNodeStatusStore();
    const pending = store.load('a');

    store.markRemoved('a');
    expect(store.data).toBeNull();
    expect(store.error).toMatchObject({ status: 404 });
    expect(store.loading).toBe(false);

    resolve(makeNodeStatus({ identity: { ...makeNodeStatus().identity, node_id: 'a' } }));
    await pending;
    expect(store.data).toBeNull();
    expect(store.error).toMatchObject({ status: 404 });
  });

  it('ignores removal events for a different node', async () => {
    const initial = makeNodeStatus({
      identity: { ...makeNodeStatus().identity, node_id: 'a' },
    });
    mockStatus.mockResolvedValueOnce(initial);
    const store = useNodeStatusStore();
    await store.load('a');

    store.markRemoved('b');
    expect(store.data).toEqual(initial);
    expect(store.error).toBeNull();
  });
});
