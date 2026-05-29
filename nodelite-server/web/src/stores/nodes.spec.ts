import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiAbortError, ApiError } from '@/api/client';
import { apiClient } from '@/api';
import { useNodesStore } from './nodes';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      listNodes: vi.fn(),
    },
  };
});

const mockListNodes = vi.mocked(apiClient.listNodes);

describe('useNodesStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockListNodes.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('populates nodes on success', async () => {
    mockListNodes.mockResolvedValueOnce([{ id: 'a' }, { id: 'b' }]);
    const store = useNodesStore();

    await store.refresh();
    expect(store.nodes).toEqual([{ id: 'a' }, { id: 'b' }]);
    expect(store.error).toBeNull();
  });

  it('captures non-abort errors', async () => {
    mockListNodes.mockRejectedValueOnce(new ApiError(503, 'down'));
    const store = useNodesStore();

    await store.refresh();
    expect(store.nodes).toEqual([]);
    expect(store.error).toBeInstanceOf(ApiError);
  });

  it('treats ApiAbortError as silent (redirect in flight)', async () => {
    mockListNodes.mockRejectedValueOnce(new ApiAbortError('redirect'));
    const store = useNodesStore();

    await store.refresh();
    expect(store.error).toBeNull();
  });

  it('skips concurrent refresh() calls', async () => {
    let resolve: (v: never[]) => void = () => {};
    mockListNodes.mockReturnValueOnce(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const store = useNodesStore();

    const first = store.refresh();
    void store.refresh();
    expect(mockListNodes).toHaveBeenCalledTimes(1);

    resolve([]);
    await first;
    expect(mockListNodes).toHaveBeenCalledTimes(1);
  });
});
