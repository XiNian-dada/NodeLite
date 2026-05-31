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
});
