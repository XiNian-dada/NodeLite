import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { apiClient } from '@/api';
import { useMonitorHistoryStore, MONITOR_MAX_POINTS, MONITOR_REFRESH_MS } from './monitorHistory';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return { ...actual, apiClient: { ...actual.apiClient, nodeHistory: vi.fn() } };
});

const mockHistory = vi.mocked(apiClient.nodeHistory);

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
    expect(mockHistory).toHaveBeenCalledWith('a', { windowHours: 24, maxPoints: MONITOR_MAX_POINTS });
  });

  it('refetches when the window changes, clearing stale points', async () => {
    mockHistory.mockResolvedValue([]);
    const store = useMonitorHistoryStore();
    await store.loadIfStale('a', 24);
    expect(mockHistory).toHaveBeenCalledTimes(1);
    await store.loadIfStale('a', 72); // window change → new key → fetch
    expect(mockHistory).toHaveBeenCalledTimes(2);
    expect(mockHistory).toHaveBeenLastCalledWith('a', { windowHours: 72, maxPoints: MONITOR_MAX_POINTS });
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
});
