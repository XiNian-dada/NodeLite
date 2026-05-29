import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type NodeSummary } from '@/api';
import { ApiAbortError } from '@/api/client';

/**
 * Node list state. Polling lifecycle is NOT owned by the store —
 * see composables/usePolling.ts. Stores hold state + refresh() only.
 */
export const useNodesStore = defineStore('nodes', () => {
  const nodes = shallowRef<NodeSummary[]>([]);
  const loading = ref(false);
  const error = ref<Error | null>(null);

  async function refresh(): Promise<void> {
    if (loading.value) return;
    loading.value = true;
    error.value = null;
    try {
      nodes.value = await apiClient.listNodes();
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      error.value = e instanceof Error ? e : new Error(String(e));
    } finally {
      loading.value = false;
    }
  }

  return { nodes, loading, error, refresh };
});
