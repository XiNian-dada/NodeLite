import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiAbortError, ApiError } from '@/api/client';
import { apiClient } from '@/api';
import { useBootstrapStore } from './bootstrap';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      bootstrap: vi.fn(),
    },
  };
});

const mockBootstrap = vi.mocked(apiClient.bootstrap);

describe('useBootstrapStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockBootstrap.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('populates data on success', async () => {
    mockBootstrap.mockResolvedValueOnce({ refreshIntervalMs: 4000 });
    const store = useBootstrapStore();

    expect(store.loading).toBe(false);
    const promise = store.load();
    expect(store.loading).toBe(true);
    await promise;

    expect(store.loading).toBe(false);
    expect(store.data).toEqual({ refreshIntervalMs: 4000 });
    expect(store.error).toBeNull();
  });

  it('captures errors and leaves data null', async () => {
    mockBootstrap.mockRejectedValueOnce(new ApiError(500, 'boom'));
    const store = useBootstrapStore();

    await store.load();
    expect(store.loading).toBe(false);
    expect(store.data).toBeNull();
    expect(store.error).toBeInstanceOf(ApiError);
  });

  it('swallows ApiAbortError silently (redirect already initiated)', async () => {
    mockBootstrap.mockRejectedValueOnce(new ApiAbortError('redirect'));
    const store = useBootstrapStore();

    await store.load();
    expect(store.error).toBeNull();
    expect(store.data).toBeNull();
  });

  it('ignores concurrent load() calls', async () => {
    let resolve: (v: { ok: true }) => void = () => {};
    mockBootstrap.mockReturnValueOnce(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const store = useBootstrapStore();

    const first = store.load();
    void store.load();
    void store.load();
    expect(mockBootstrap).toHaveBeenCalledTimes(1);

    resolve({ ok: true });
    await first;
    expect(mockBootstrap).toHaveBeenCalledTimes(1);
  });
});
