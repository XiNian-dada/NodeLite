import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type BootstrapResponse } from '@/api';
import { ApiAbortError } from '@/api/client';

/**
 * Bootstrap state — one-shot fetch from /api/bootstrap, no polling.
 * The polling cadence (REFRESH_MS) comes from this response when present.
 */
export const useBootstrapStore = defineStore('bootstrap', () => {
  const data = shallowRef<BootstrapResponse | null>(null);
  const loading = ref(false);
  const error = ref<Error | null>(null);

  async function load(): Promise<void> {
    if (loading.value) return;
    loading.value = true;
    error.value = null;
    try {
      data.value = await apiClient.bootstrap();
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      error.value = e instanceof Error ? e : new Error(String(e));
    } finally {
      loading.value = false;
    }
  }

  return { data, loading, error, load };
});
