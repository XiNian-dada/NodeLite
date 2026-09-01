import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import { apiClient, type NodeListItem, type NodeStatus } from '@/api';
import { ApiAbortError, ApiError } from '@/api/client';
import { useDedupeAsync } from '@/composables/useDedupeAsync';

/**
 * Active node's full status (GET /api/nodes/{id}). Single active node — the
 * NodeDetail view loads the full record once, then merges lightweight
 * WebSocket summaries into the realtime fields. If the id changes, data is
 * cleared so a stale node's snapshot never flashes under the new id.
 */
export const useNodeStatusStore = defineStore('nodeStatus', () => {
  const nodeId = ref<string | null>(null);
  const data = shallowRef<NodeStatus | null>(null);
  const loading = ref(false);
  const error = ref<Error | null>(null);
  const requests = useDedupeAsync<string>();

  async function fetchFor(id: string): Promise<void> {
    await requests.run(id, async ({ isCurrent }) => {
      loading.value = true;
      error.value = null;
      try {
        const result = await apiClient.nodeStatus(id);
        // Discard a late response for a node we've since navigated away from.
        if (isCurrent() && nodeId.value === id) data.value = result;
      } catch (e) {
        if (e instanceof ApiAbortError) return;
        if (isCurrent() && nodeId.value === id) {
          error.value = e instanceof Error ? e : new Error(String(e));
        }
      } finally {
        if (isCurrent()) {
          loading.value = false;
        }
      }
    });
  }

  /** Switch to a node: clears stale data if the id changed, then fetches. */
  async function load(id: string): Promise<void> {
    if (nodeId.value !== id) {
      nodeId.value = id;
      data.value = null;
      error.value = null;
    }
    await fetchFor(id);
  }

  /** Re-fetch the current node. No-op if no node is active. */
  async function refresh(): Promise<void> {
    if (nodeId.value === null) return;
    await fetchFor(nodeId.value);
  }

  function applyRealtimeSummary(summary: NodeListItem, generatedAt: string): void {
    const current = data.value;
    if (!current || nodeId.value !== summary.identity.node_id) return;

    const snapshot = summary.snapshot
      ? {
          ...current.snapshot,
          collected_at: current.snapshot?.collected_at ?? generatedAt,
          cpu_usage_percent: summary.snapshot.cpu_usage_percent,
          load: {
            one: summary.snapshot.load.one,
            five: current.snapshot?.load.five ?? summary.snapshot.load.one,
            fifteen: current.snapshot?.load.fifteen ?? summary.snapshot.load.one,
          },
          memory: {
            total_bytes: summary.snapshot.memory.total_bytes,
            used_bytes: summary.snapshot.memory.used_bytes,
            available_bytes: Math.max(
              summary.snapshot.memory.total_bytes - summary.snapshot.memory.used_bytes,
              0,
            ),
            swap_total_bytes: current.snapshot?.memory.swap_total_bytes ?? 0,
            swap_used_bytes: current.snapshot?.memory.swap_used_bytes ?? 0,
          },
          uptime_secs: current.snapshot?.uptime_secs ?? 0,
          disks: current.snapshot?.disks ?? [],
          network: current.snapshot?.network ?? {
            total_rx_bytes: 0,
            total_tx_bytes: 0,
            rx_bytes_per_sec: null,
            tx_bytes_per_sec: null,
            packet_loss_percent: null,
          },
        }
      : null;

    data.value = {
      ...current,
      identity: {
        ...current.identity,
        node_label: summary.identity.node_label,
        hostname: summary.identity.hostname,
        tags: summary.identity.tags,
      },
      geoip_country: summary.geoip_country,
      geoip_city: summary.geoip_city,
      geoip_latitude: summary.geoip_latitude,
      geoip_longitude: summary.geoip_longitude,
      location_override_country: summary.location_override_country,
      location_override_city: summary.location_override_city,
      location_override_latitude: summary.location_override_latitude,
      location_override_longitude: summary.location_override_longitude,
      snapshot,
      latency_ms: summary.latency_ms,
      online: summary.online,
    };
  }

  function markRemoved(removedNodeId: string): void {
    if (nodeId.value !== removedNodeId) return;
    requests.abort();
    data.value = null;
    loading.value = false;
    error.value = new ApiError(404, 'node removed');
  }

  return { nodeId, data, loading, error, load, refresh, applyRealtimeSummary, markRemoved };
});
