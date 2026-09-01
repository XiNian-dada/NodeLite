import { defineStore } from 'pinia';
import { ref, shallowRef, triggerRef } from 'vue';
import { apiClient, type NodeListItem } from '@/api';
import { ApiAbortError } from '@/api/client';

/**
 * Node list state. A Map and index table keep WebSocket upserts O(1), while a
 * shallow array preserves iteration order without rebuilding via Array.from
 * for every message. Polling lifecycle is NOT owned by the store — see
 * composables/usePolling.ts. Stores hold state + refresh() only.
 *
 * Timestamp guard: single global `lastGeneratedAt` protects against stale
 * messages (e.g., a delayed incremental arriving after a fresh InitialState).
 * This is correct because messages share one ordered WS connection. If we
 * ever introduce concurrent channels (Web Worker, per-node sub-channels),
 * revisit this — a single global would silently drop legitimate concurrent
 * updates.
 */
export const useNodesStore = defineStore('nodes', () => {
  const nodes = shallowRef<NodeListItem[]>([]);
  const nodesById = shallowRef<Map<string, NodeListItem>>(new Map());
  const nodeIndexById = new Map<string, number>();
  const lastGeneratedAt = ref<string | null>(null);
  const loading = ref(false);
  const error = ref<Error | null>(null);

  async function refresh(): Promise<void> {
    if (loading.value) return;
    loading.value = true;
    error.value = null;
    try {
      const result = await apiClient.listNodes();
      // Use current server-side baseline if available, else fall back to client clock
      const timestamp = lastGeneratedAt.value || new Date().toISOString();
      applyServerState(result, timestamp);
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      error.value = e instanceof Error ? e : new Error(String(e));
    } finally {
      loading.value = false;
    }
  }

  // From WS InitialState (full replacement) — always accept, no guard
  function applyServerState(items: NodeListItem[], generatedAt: string): void {
    const next = new Map<string, NodeListItem>();
    for (const item of items) next.set(item.identity.node_id, item);
    const nextItems = Array.from(next.values());
    nodeIndexById.clear();
    nextItems.forEach((item, index) => {
      nodeIndexById.set(item.identity.node_id, index);
    });
    nodes.value = nextItems;
    nodesById.value = next;
    lastGeneratedAt.value = generatedAt;
  }

  // From WS NodeUpsert
  function upsertNode(node: NodeListItem, generatedAt: string): void {
    if (lastGeneratedAt.value && Date.parse(generatedAt) < Date.parse(lastGeneratedAt.value))
      return;
    const nodeId = node.identity.node_id;
    const index = nodeIndexById.get(nodeId);
    if (index === undefined) {
      nodeIndexById.set(nodeId, nodes.value.length);
      nodes.value.push(node);
    } else {
      nodes.value[index] = node;
    }
    nodesById.value.set(nodeId, node);
    triggerRef(nodes);
    triggerRef(nodesById);
    lastGeneratedAt.value = generatedAt;
  }

  // From WS NodeRemoved
  function removeNode(nodeId: string, generatedAt: string): boolean {
    if (lastGeneratedAt.value && Date.parse(generatedAt) < Date.parse(lastGeneratedAt.value)) {
      return false;
    }
    const index = nodeIndexById.get(nodeId);
    if (index !== undefined) {
      nodes.value.splice(index, 1);
      nodeIndexById.delete(nodeId);
      for (let nextIndex = index; nextIndex < nodes.value.length; nextIndex++) {
        const nextNode = nodes.value[nextIndex];
        if (nextNode) nodeIndexById.set(nextNode.identity.node_id, nextIndex);
      }
      triggerRef(nodes);
    }
    if (nodesById.value.delete(nodeId)) triggerRef(nodesById);
    lastGeneratedAt.value = generatedAt;
    return true;
  }

  return {
    nodes,
    nodesById,
    lastGeneratedAt,
    loading,
    error,
    refresh,
    applyServerState,
    upsertNode,
    removeNode,
  };
});
