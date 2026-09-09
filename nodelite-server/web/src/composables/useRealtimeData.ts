import { onMounted, onUnmounted } from 'vue';
import { useNodesStore } from '@/stores/nodes';
import { useOverviewStore } from '@/stores/overview';
import { useRealtimeStore } from '@/stores/realtime';
import { useWebSocket } from '@/ws';

/** App owns both the connection and its consumers so route changes cannot drop updates. */
export function useRealtimeData(): void {
  const ws = useWebSocket();
  const nodes = useNodesStore();
  const overview = useOverviewStore();
  const realtime = useRealtimeStore();
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let request: AbortController | null = null;
  let retryDelay = 5000;

  function clearPoll(): void {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  function schedulePoll(delay = 500): void {
    if (disposed || realtime.streaming || document.hidden || request || timer !== null) return;
    timer = setTimeout(() => {
      timer = null;
      void poll();
    }, delay);
  }

  async function poll(): Promise<void> {
    if (disposed || realtime.streaming || document.hidden || request) return;
    const controller = new AbortController();
    request = controller;
    const deadline = setTimeout(() => controller.abort(), 10_000);
    try {
      await Promise.all([nodes.refresh(controller.signal), overview.refresh(controller.signal)]);
      if (disposed || realtime.streaming || controller.signal.aborted) return;
      if (!nodes.error && !overview.error) {
        realtime.lastUpdatedAt = Date.now();
        retryDelay = 5000;
      } else {
        retryDelay = Math.min(retryDelay * 2, 30_000);
      }
    } finally {
      clearTimeout(deadline);
      request = null;
      schedulePoll(retryDelay);
    }
  }

  function visibilityChanged(): void {
    if (document.hidden) {
      clearPoll();
      request?.abort();
    } else {
      schedulePoll();
    }
  }

  const unsubscribe = [
    ws.on('initial_state', (message) => {
      nodes.applyServerState(message.nodes, message.generated_at);
      overview.apply(message.overview, message.generated_at);
      realtime.streaming = true;
      realtime.lastUpdatedAt = Date.now();
      retryDelay = 5000;
      clearPoll();
      request?.abort();
    }),
    ws.on('node_upsert', (message) => {
      nodes.upsertNode(message.node, message.generated_at);
      realtime.lastUpdatedAt = Date.now();
    }),
    ws.on('node_removed', (message) => {
      nodes.removeNode(message.node_id, message.generated_at);
      realtime.lastUpdatedAt = Date.now();
    }),
    ws.on('overview_update', (message) => {
      overview.apply(message.overview, message.generated_at);
      realtime.lastUpdatedAt = Date.now();
    }),
    ws.onState((state) => {
      realtime.connection = state;
      realtime.streaming = false;
      schedulePoll();
    }),
  ];

  const freshnessTimer = setInterval(() => {
    realtime.now = Date.now();
  }, 1000);
  onMounted(() => {
    document.addEventListener('visibilitychange', visibilityChanged);
    ws.connect();
  });
  onUnmounted(() => {
    disposed = true;
    clearPoll();
    clearInterval(freshnessTimer);
    request?.abort();
    document.removeEventListener('visibilitychange', visibilityChanged);
    for (const off of unsubscribe) off();
    ws.destroy();
  });
}
