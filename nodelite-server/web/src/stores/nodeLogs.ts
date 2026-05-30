import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type AgentLogEntry } from '@/api';
import { ApiAbortError } from '@/api/client';

export const NODE_LOGS_LIMIT = 200;
export const NODE_LOGS_REFRESH_MS = 10 * 1000;

/**
 * Active node's agent logs (GET /api/nodes/{id}/logs?limit=200). Same
 * single-active-node pattern as detailHistory: id-switch clears stale
 * entries, id-aware in-flight guard, loadIfStale throttles tab re-entry,
 * refresh() polls at >=10s.
 */
export const useNodeLogsStore = defineStore('nodeLogs', () => {
  const nodeId = ref<string | null>(null);
  const entries = shallowRef<AgentLogEntry[]>([]);
  const loading = ref(false);
  const error = ref<Error | null>(null);
  const fetchedAt = ref(0);
  const inFlightId = ref<string | null>(null);

  async function fetchFor(id: string): Promise<void> {
    if (inFlightId.value === id) return;
    inFlightId.value = id;
    loading.value = true;
    error.value = null;
    try {
      const result = await apiClient.nodeLogs(id, NODE_LOGS_LIMIT);
      if (nodeId.value === id) {
        entries.value = result;
        fetchedAt.value = Date.now();
      }
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      if (nodeId.value === id) {
        error.value = e instanceof Error ? e : new Error(String(e));
        fetchedAt.value = Date.now();
      }
    } finally {
      if (inFlightId.value === id) {
        inFlightId.value = null;
        loading.value = false;
      }
    }
  }

  function switchTo(id: string): boolean {
    if (nodeId.value === id) return false;
    nodeId.value = id;
    entries.value = [];
    error.value = null;
    fetchedAt.value = 0;
    return true;
  }

  async function loadIfStale(id: string): Promise<void> {
    if (switchTo(id)) {
      await fetchFor(id);
      return;
    }
    if (fetchedAt.value > 0 && Date.now() - fetchedAt.value < NODE_LOGS_REFRESH_MS) return;
    await fetchFor(id);
  }

  async function refresh(): Promise<void> {
    const id = nodeId.value;
    if (id === null) return;
    if (fetchedAt.value > 0 && Date.now() - fetchedAt.value < NODE_LOGS_REFRESH_MS) return;
    await fetchFor(id);
  }

  return { nodeId, entries, loading, error, loadIfStale, refresh };
});
