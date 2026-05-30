import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type HistoryPoint } from '@/api';
import { ApiAbortError } from '@/api/client';

/** High-res monitor history (node.html fetchDetailHistory max_points=720). */
export const MONITOR_MAX_POINTS = 720;
export const MONITOR_REFRESH_MS = 15 * 1000;

function keyOf(id: string, windowHours: number): string {
  return `${id}:${windowHours}`;
}

/**
 * Monitor-tab history for the active (node, window) pair. The selected
 * preset changes windowHours, which is part of the cache key — switching
 * window or node clears stale points and refetches; same key only refetches
 * once >=15s stale. id/key-aware in-flight guard (the #182 lesson).
 */
export const useMonitorHistoryStore = defineStore('monitorHistory', () => {
  const currentKey = ref<string | null>(null);
  const points = shallowRef<HistoryPoint[]>([]);
  const loading = ref(false);
  const error = ref<Error | null>(null);
  const fetchedAt = ref(0);
  const inFlightKey = ref<string | null>(null);

  async function fetchFor(id: string, windowHours: number, key: string): Promise<void> {
    if (inFlightKey.value === key) return;
    inFlightKey.value = key;
    loading.value = true;
    error.value = null;
    try {
      const result = await apiClient.nodeHistory(id, { windowHours, maxPoints: MONITOR_MAX_POINTS });
      if (currentKey.value === key) {
        points.value = result;
        fetchedAt.value = Date.now();
      }
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      if (currentKey.value === key) {
        error.value = e instanceof Error ? e : new Error(String(e));
        fetchedAt.value = Date.now();
      }
    } finally {
      if (inFlightKey.value === key) {
        inFlightKey.value = null;
        loading.value = false;
      }
    }
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
  async function loadIfStale(id: string, windowHours: number): Promise<void> {
    const key = keyOf(id, windowHours);
    if (switchTo(key)) {
      await fetchFor(id, windowHours, key);
      return;
    }
    if (fetchedAt.value > 0 && Date.now() - fetchedAt.value < MONITOR_REFRESH_MS) return;
    await fetchFor(id, windowHours, key);
  }

  /** Poll the current pair, throttled to >=15s. */
  async function refresh(id: string, windowHours: number): Promise<void> {
    const key = keyOf(id, windowHours);
    if (currentKey.value !== key) {
      await loadIfStale(id, windowHours);
      return;
    }
    if (fetchedAt.value > 0 && Date.now() - fetchedAt.value < MONITOR_REFRESH_MS) return;
    await fetchFor(id, windowHours, key);
  }

  return { currentKey, points, loading, error, loadIfStale, refresh };
});
