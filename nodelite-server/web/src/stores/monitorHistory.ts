import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type HistoryPoint } from '@/api';
import { ApiAbortError } from '@/api/client';
import { useDedupeAsync } from '@/composables/useDedupeAsync';

/** High-res monitor history (node.html fetchDetailHistory max_points=720). */
export const MONITOR_MAX_POINTS = 720;
export const MONITOR_REFRESH_MS = 15 * 1000;

export type HistoryWindow = number | { start: number; end: number };

function keyOf(id: string, window: HistoryWindow): string {
  return typeof window === 'number' ? `${id}:${window}` : `${id}:${window.start}:${window.end}`;
}

/**
 * High-res history for the active node and live window or explicit range.
 * The complete query is part of the cache key — switching
 * window or node clears stale points and refetches; same key only refetches
 * once >=15s stale. Writes are owned per request, not per key: after A→B→A the
 * key matches again, so comparing keys would revive the first A response.
 */
export const useMonitorHistoryStore = defineStore('monitorHistory', () => {
  const currentKey = ref<string | null>(null);
  const points = shallowRef<HistoryPoint[]>([]);
  const loading = ref(false);
  const error = ref<Error | null>(null);
  const fetchedAt = ref(0);
  const requests = useDedupeAsync<string>();

  async function fetchFor(id: string, window: HistoryWindow, key: string): Promise<void> {
    await requests.run(key, async ({ isCurrent }) => {
      loading.value = true;
      error.value = null;
      try {
        const query = typeof window === 'number' ? { windowHours: window } : window;
        const result = await apiClient.nodeHistory(id, { ...query, maxPoints: MONITOR_MAX_POINTS });
        if (isCurrent()) {
          points.value = result;
          fetchedAt.value = Date.now();
        }
      } catch (e) {
        if (e instanceof ApiAbortError) return;
        if (isCurrent()) {
          error.value = e instanceof Error ? e : new Error(String(e));
          fetchedAt.value = Date.now();
        }
      } finally {
        if (isCurrent()) {
          loading.value = false;
        }
      }
    });
  }

  function switchTo(key: string): boolean {
    if (currentKey.value === key) return false;
    currentKey.value = key;
    points.value = [];
    error.value = null;
    fetchedAt.value = 0;
    return true;
  }

  /** Load for (node, window); a new pair always fetches, same pair throttles. */
  async function loadIfStale(id: string, window: HistoryWindow): Promise<void> {
    const key = keyOf(id, window);
    if (switchTo(key)) {
      await fetchFor(id, window, key);
      return;
    }
    if (fetchedAt.value > 0 && Date.now() - fetchedAt.value < MONITOR_REFRESH_MS) return;
    await fetchFor(id, window, key);
  }

  /** Poll the current pair, throttled to >=15s. */
  async function refresh(id: string, window: HistoryWindow): Promise<void> {
    const key = keyOf(id, window);
    if (currentKey.value !== key) {
      await loadIfStale(id, window);
      return;
    }
    if (fetchedAt.value > 0 && Date.now() - fetchedAt.value < MONITOR_REFRESH_MS) return;
    await fetchFor(id, window, key);
  }

  return { currentKey, points, loading, error, loadIfStale, refresh };
});
