import { setActivePinia, createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiAbortError, ApiError } from '@/api/client';
import { apiClient, type UpdateAlertSettingsRequest } from '@/api';
import { makeAlertSettings } from '@/api/__fixtures__/nodes';
import { useAlertsStore } from './alerts';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, alertSettings: vi.fn(), updateAlertSettings: vi.fn() },
  };
});

const mockGet = vi.mocked(apiClient.alertSettings);
const mockPost = vi.mocked(apiClient.updateAlertSettings);

const EMPTY_PAYLOAD = {} as UpdateAlertSettingsRequest;

describe('useAlertsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockGet.mockReset();
    mockPost.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('loads config + preview on success', async () => {
    const res = makeAlertSettings({ config: { enabled: false } });
    mockGet.mockResolvedValueOnce(res);
    const store = useAlertsStore();
    await store.load();
    expect(store.config).toEqual(res.config);
    expect(store.preview).toEqual(res.preview);
    expect(store.error).toBeNull();
  });

  it('captures non-abort load errors', async () => {
    mockGet.mockRejectedValueOnce(new ApiError(503, 'down'));
    const store = useAlertsStore();
    await store.load();
    expect(store.config).toBeNull();
    expect(store.error).toBeInstanceOf(ApiError);
  });

  it('swallows ApiAbortError on load', async () => {
    mockGet.mockRejectedValueOnce(new ApiAbortError('redirect'));
    const store = useAlertsStore();
    await store.load();
    expect(store.error).toBeNull();
  });

  it('skips concurrent loads', async () => {
    let resolve: (v: ReturnType<typeof makeAlertSettings>) => void = () => {};
    mockGet.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    const store = useAlertsStore();
    const first = store.load();
    void store.load();
    expect(mockGet).toHaveBeenCalledTimes(1);
    resolve(makeAlertSettings());
    await first;
    expect(mockGet).toHaveBeenCalledTimes(1);
  });

  it('save refreshes config + preview from the POST response', async () => {
    const updated = makeAlertSettings({ config: { enabled: true } });
    mockPost.mockResolvedValueOnce(updated);
    const store = useAlertsStore();
    await store.save(EMPTY_PAYLOAD);
    expect(mockPost).toHaveBeenCalledWith(EMPTY_PAYLOAD);
    expect(store.config).toEqual(updated.config);
    expect(store.saving).toBe(false);
  });

  it('save propagates the error and clears the saving flag', async () => {
    mockPost.mockRejectedValueOnce(new ApiError(401, JSON.stringify({ ok: false, message: 'bad code' })));
    const store = useAlertsStore();
    await expect(store.save(EMPTY_PAYLOAD)).rejects.toBeInstanceOf(ApiError);
    expect(store.saving).toBe(false);
  });

  describe('load/save ordering', () => {
    const stale = makeAlertSettings({ config: { enabled: false } });
    const saved = makeAlertSettings({ config: { enabled: true } });

    function deferred<T>() {
      let resolve: (value: T) => void = () => {};
      let reject: (error: Error) => void = () => {};
      const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
      });
      return { promise, resolve, reject };
    }

    it('drops a GET sent before the POST when it returns after the POST', async () => {
      const get = deferred<typeof stale>();
      mockGet.mockReturnValueOnce(get.promise);
      mockPost.mockResolvedValueOnce(saved);
      const store = useAlertsStore();

      const loading = store.load();
      await store.save(EMPTY_PAYLOAD);
      get.resolve(stale);
      await loading;

      expect(store.config?.enabled).toBe(true);
      expect(store.loading).toBe(false);
    });

    it('keeps the POST result when the GET returns before the POST', async () => {
      const get = deferred<typeof stale>();
      const post = deferred<typeof saved>();
      mockGet.mockReturnValueOnce(get.promise);
      mockPost.mockReturnValueOnce(post.promise);
      const store = useAlertsStore();

      const loading = store.load();
      const saving = store.save(EMPTY_PAYLOAD);
      get.resolve(stale);
      await loading;
      post.resolve(saved);
      await saving;

      expect(store.config?.enabled).toBe(true);
    });

    it('drops a GET sent while the POST is in flight once the POST commits', async () => {
      const get = deferred<typeof stale>();
      const post = deferred<typeof saved>();
      mockGet.mockReturnValueOnce(get.promise);
      mockPost.mockReturnValueOnce(post.promise);
      const store = useAlertsStore();

      const saving = store.save(EMPTY_PAYLOAD);
      const loading = store.load();
      post.resolve(saved);
      await saving;
      get.resolve(stale);
      await loading;

      expect(store.config?.enabled).toBe(true);
    });

    it('does not report a failed GET that a save already superseded', async () => {
      const get = deferred<typeof stale>();
      mockGet.mockReturnValueOnce(get.promise);
      mockPost.mockResolvedValueOnce(saved);
      const store = useAlertsStore();

      const loading = store.load();
      await store.save(EMPTY_PAYLOAD);
      get.reject(new ApiError(503, 'down'));
      await loading;

      expect(store.error).toBeNull();
    });

    it('lets a concurrent caller wait for the shared load', async () => {
      const get = deferred<typeof stale>();
      mockGet.mockReturnValueOnce(get.promise);
      const store = useAlertsStore();

      void store.load();
      const second = store.load();
      get.resolve(saved);
      await second;

      expect(store.config?.enabled).toBe(true);
      expect(mockGet).toHaveBeenCalledTimes(1);
    });
  });
});
