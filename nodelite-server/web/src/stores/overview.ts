import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type OverviewData } from '@/api';
import { ApiAbortError } from '@/api/client';

/**
 * Overview aggregate stats. Polling lifecycle is NOT owned by the store —
 * see composables/usePolling.ts. Stores hold state + refresh() only.
 *
 * Timestamp guard: single global `lastGeneratedAt` protects against stale
 * messages (e.g., a delayed update arriving after a fresh InitialState).
 */
export const useOverviewStore = defineStore('overview', () => {
  const data = shallowRef<OverviewData | null>(null);
  const lastGeneratedAt = ref<string | null>(null);
  const loading = ref(false);
  const error = ref<Error | null>(null);
  let revision = 0;

  async function refresh(signal?: AbortSignal): Promise<void> {
    if (loading.value) return;
    loading.value = true;
    error.value = null;
    const startedAtRevision = revision;
    try {
      const result = await apiClient.overview(signal);
      if (signal?.aborted || revision !== startedAtRevision) return;
      apply(result, result.generated_at);
    } catch (e) {
      if (signal?.aborted || e instanceof ApiAbortError) return;
      error.value = e instanceof Error ? e : new Error(String(e));
    } finally {
      loading.value = false;
    }
  }

  // From WS InitialState or OverviewUpdate
  function apply(overview: OverviewData, generatedAt: string): void {
    if (lastGeneratedAt.value && Date.parse(generatedAt) < Date.parse(lastGeneratedAt.value))
      return;
    revision++;
    data.value = overview;
    lastGeneratedAt.value = generatedAt;
  }

  return { data, lastGeneratedAt, loading, error, refresh, apply };
});
