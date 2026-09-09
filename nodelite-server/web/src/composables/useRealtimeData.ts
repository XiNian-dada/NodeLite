import { onMounted, onUnmounted } from 'vue';
import { useNodesStore } from '@/stores/nodes';
import { useOverviewStore } from '@/stores/overview';
import { useWebSocket } from '@/ws';

/** App owns both the connection and its consumers so route changes cannot drop updates. */
export function useRealtimeData(): void {
  const ws = useWebSocket();
  const nodes = useNodesStore();
  const overview = useOverviewStore();
  const unsubscribe = [
    ws.on('initial_state', (message) => {
      nodes.applyServerState(message.nodes, message.generated_at);
      overview.apply(message.overview, message.generated_at);
    }),
    ws.on('node_upsert', (message) => nodes.upsertNode(message.node, message.generated_at)),
    ws.on('node_removed', (message) => nodes.removeNode(message.node_id, message.generated_at)),
    ws.on('overview_update', (message) => overview.apply(message.overview, message.generated_at)),
  ];

  onMounted(() => ws.connect());
  onUnmounted(() => {
    for (const off of unsubscribe) off();
    ws.destroy();
  });
}
