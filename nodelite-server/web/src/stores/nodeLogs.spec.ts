import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiAbortError, ApiError } from '@/api/client';
import { apiClient } from '@/api';
import { makeLogEntry } from '@/api/__fixtures__/nodes';
import { useNodeLogsStore, NODE_LOGS_LIMIT, NODE_LOGS_REFRESH_MS } from './nodeLogs';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return { ...actual, apiClient: { ...actual.apiClient, nodeLogs: vi.fn() } };
});

const mockLogs = vi.mocked(apiClient.nodeLogs);

describe('useNodeLogsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockLogs.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it('loads logs with limit 200', async () => {
    mockLogs.mockResolvedValueOnce([makeLogEntry()]);
    const store = useNodeLogsStore();
    await store.loadIfStale('a');
    expect(mockLogs).toHaveBeenCalledWith('a', NODE_LOGS_LIMIT);
    expect(store.entries.length).toBe(1);
  });

  it('throttles same-node re-entry but fetches on switch', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    mockLogs.mockResolvedValue([]);
    const store = useNodeLogsStore();

    await store.loadIfStale('a');
    expect(mockLogs).toHaveBeenCalledTimes(1);

    vi.setSystemTime(1_000_000 + NODE_LOGS_REFRESH_MS - 1);
    await store.loadIfStale('a');
    expect(mockLogs).toHaveBeenCalledTimes(1);

    await store.loadIfStale('b');
    expect(mockLogs).toHaveBeenCalledTimes(2);
  });

  it('clears stale entries when switching nodes', async () => {
    mockLogs.mockResolvedValueOnce([makeLogEntry()]);
    const store = useNodeLogsStore();
    await store.loadIfStale('a');
    expect(store.entries.length).toBe(1);

    let resolve: (v: never[]) => void = () => {};
    mockLogs.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    const pending = store.loadIfStale('b');
    expect(store.entries).toEqual([]);
    resolve([]);
    await pending;
  });

  it('records non-abort errors and swallows ApiAbortError', async () => {
    mockLogs.mockRejectedValueOnce(new ApiError(503, 'down'));
    const store = useNodeLogsStore();
    await store.loadIfStale('a');
    expect(store.error).toBeInstanceOf(ApiError);

    setActivePinia(createPinia());
    const s2 = useNodeLogsStore();
    mockLogs.mockRejectedValueOnce(new ApiAbortError('redirect'));
    await s2.loadIfStale('a');
    expect(s2.error).toBeNull();
  });

  it('refresh is a no-op with no active node', async () => {
    const store = useNodeLogsStore();
    await store.refresh();
    expect(mockLogs).not.toHaveBeenCalled();
  });
});
