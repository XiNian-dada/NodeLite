import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { apiClient, type HistoryPoint } from '@/api';
import { useMonitorHistoryStore, MONITOR_MAX_POINTS, MONITOR_REFRESH_MS } from './monitorHistory';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return { ...actual, apiClient: { ...actual.apiClient, nodeHistory: vi.fn() } };
});

const mockHistory = vi.mocked(apiClient.nodeHistory);

interface Deferred {
  promise: Promise<HistoryPoint[]>;
  resolve: (points: HistoryPoint[]) => void;
  reject: (error: Error) => void;
}

function deferred(): Deferred {
  let resolve: Deferred['resolve'] = () => {};
  let reject: Deferred['reject'] = () => {};
  const promise = new Promise<HistoryPoint[]>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function queueRequests(count: number): Deferred[] {
  const requests = Array.from({ length: count }, deferred);
  for (const request of requests) mockHistory.mockReturnValueOnce(request.promise);
  return requests;
}

const OLD = [{ ts: 'old' }] as unknown as HistoryPoint[];
const NEW = [{ ts: 'new' }] as unknown as HistoryPoint[];

describe('useMonitorHistoryStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockHistory.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it('fetches high-res history for the (node, window) pair', async () => {
    mockHistory.mockResolvedValueOnce([]);
    const store = useMonitorHistoryStore();
    await store.loadIfStale('a', 24);
    expect(mockHistory).toHaveBeenCalledWith('a', {
      windowHours: 24,
      maxPoints: MONITOR_MAX_POINTS,
    });
  });

  it('refetches when the window changes, clearing stale points', async () => {
    mockHistory.mockResolvedValue([]);
    const store = useMonitorHistoryStore();
    await store.loadIfStale('a', 24);
    expect(mockHistory).toHaveBeenCalledTimes(1);
    await store.loadIfStale('a', 72); // window change → new key → fetch
    expect(mockHistory).toHaveBeenCalledTimes(2);
    expect(mockHistory).toHaveBeenLastCalledWith('a', {
      windowHours: 72,
      maxPoints: MONITOR_MAX_POINTS,
    });
  });

  it('throttles same-pair re-entry within the window', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    mockHistory.mockResolvedValue([]);
    const store = useMonitorHistoryStore();
    await store.loadIfStale('a', 24);
    vi.setSystemTime(1_000_000 + MONITOR_REFRESH_MS - 1);
    await store.loadIfStale('a', 24);
    expect(mockHistory).toHaveBeenCalledTimes(1);
    vi.setSystemTime(1_000_000 + MONITOR_REFRESH_MS + 1);
    await store.loadIfStale('a', 24);
    expect(mockHistory).toHaveBeenCalledTimes(2);
  });

  it('refresh loads when the current pair differs', async () => {
    mockHistory.mockResolvedValue([]);
    const store = useMonitorHistoryStore();
    await store.refresh('a', 24);
    expect(mockHistory).toHaveBeenCalledTimes(1);
  });

  it('loads precise navigator ranges without mixing them with live hourly queries', async () => {
    mockHistory.mockResolvedValue([]);
    const store = useMonitorHistoryStore();
    await store.loadIfStale('a', 1);
    await store.loadIfStale('a', { start: 1000, end: 2000 });
    expect(mockHistory).toHaveBeenLastCalledWith('a', {
      start: 1000,
      end: 2000,
      maxPoints: MONITOR_MAX_POINTS,
    });
    await store.loadIfStale('a', 1);
    expect(mockHistory).toHaveBeenLastCalledWith('a', {
      windowHours: 1,
      maxPoints: MONITOR_MAX_POINTS,
    });
  });

  describe.each([
    ['node', ['a', 1], ['b', 1]],
    ['window', ['a', 1], ['a', 24]],
  ] as [string, [string, number], [string, number]][])(
    'A→B→A %s switching',
    (_name, first, second) => {
      async function switchBack(store: ReturnType<typeof useMonitorHistoryStore>) {
        const pending = [
          store.loadIfStale(...first),
          store.loadIfStale(...second),
          store.loadIfStale(...first),
        ];
        expect(mockHistory).toHaveBeenCalledTimes(3);
        return pending;
      }

      it('keeps the newer result when the first request finishes last', async () => {
        const [a1, b1, a2] = queueRequests(3);
        const store = useMonitorHistoryStore();
        const pending = await switchBack(store);

        a2?.resolve(NEW);
        await pending[2];
        expect(store.points).toBe(NEW);
        expect(store.loading).toBe(false);

        a1?.resolve(OLD);
        b1?.resolve(OLD);
        await Promise.all(pending);
        expect(store.points).toBe(NEW);
        expect(store.loading).toBe(false);
      });

      it('ignores a stale failure and its finally while the new request is pending', async () => {
        const [a1, b1, a2] = queueRequests(3);
        const store = useMonitorHistoryStore();
        const pending = await switchBack(store);

        a1?.reject(new Error('stale'));
        b1?.resolve(OLD);
        await Promise.all(pending.slice(0, 2));
        expect(store.error).toBeNull();
        expect(store.points).toEqual([]);
        expect(store.loading).toBe(true);

        // The pending guard must survive too, or polling would duplicate the request.
        const poll = store.refresh(...first);
        expect(mockHistory).toHaveBeenCalledTimes(3);

        a2?.resolve(NEW);
        await Promise.all([pending[2], poll]);
        expect(store.points).toBe(NEW);
        expect(store.loading).toBe(false);
      });
    },
  );
});
