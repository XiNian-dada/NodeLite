<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue';
import AppLayout from '@/components/AppLayout.vue';
import OverviewStats from '@/components/OverviewStats.vue';
import NodeHealthMatrix from '@/components/NodeHealthMatrix.vue';
import NodeMap from '@/components/NodeMap.vue';
import NodeList from '@/components/NodeList.vue';
import { useWebSocket } from '@/ws';
import { useBootstrapStore } from '@/stores/bootstrap';
import { useOverviewStore } from '@/stores/overview';
import { useNodesStore } from '@/stores/nodes';

const bootstrapStore = useBootstrapStore();
const overviewStore = useOverviewStore();
const nodesStore = useNodesStore();
const ws = useWebSocket();
const DASHBOARD_REST_FALLBACK_MS = 500;

const onlineCount = computed(() => overviewStore.data?.online_nodes ?? 0);

onMounted(() => {
  void bootstrapStore.load();

  // WS-first: subscribe to WebSocket messages
  const offInitial = ws.on('initial_state', (msg) => {
    overviewStore.apply(msg.overview, msg.generated_at);
    nodesStore.applyServerState(msg.nodes, msg.generated_at);
  });

  const offOverview = ws.on('overview_update', (msg) => {
    overviewStore.apply(msg.overview, msg.generated_at);
  });

  const offUpsert = ws.on('node_upsert', (msg) => {
    nodesStore.upsertNode(msg.node, msg.generated_at);
  });

  const offRemoved = ws.on('node_removed', (msg) => {
    nodesStore.removeNode(msg.node_id, msg.generated_at);
  });

  // Fallback quickly so the dashboard does not sit in an empty shell while
  // the websocket reconnects; later WS messages still replace this baseline.
  const fallbackTimer = window.setTimeout(() => {
    if (!nodesStore.lastGeneratedAt) {
      void Promise.all([overviewStore.refresh(), nodesStore.refresh()]);
    }
  }, DASHBOARD_REST_FALLBACK_MS);

  onUnmounted(() => {
    offInitial();
    offOverview();
    offUpsert();
    offRemoved();
    window.clearTimeout(fallbackTimer);
  });
});
</script>

<template>
  <AppLayout>
    <template #title>
      <h1 class="dash-title">{{ $t('index.heading') }}</h1>
      <p class="dash-subtitle">{{ $t('index.subtitle', { count: onlineCount }) }}</p>
    </template>

    <section class="overview" data-test="dashboard-view">
      <OverviewStats />

      <section class="dashboard-grid" data-test="dashboard-top-row">
        <NodeMap />
        <NodeHealthMatrix />
      </section>

      <NodeList />
    </section>
  </AppLayout>
</template>

<style scoped>
.overview {
  display: grid;
  gap: 16px;
}
.dashboard-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.55fr) minmax(380px, 0.85fr);
  gap: 16px;
}
.dash-title {
  margin: 0;
  font-size: 28px;
  font-weight: 600;
  letter-spacing: 0;
}
.dash-subtitle {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 14px;
}
@media (max-width: 1320px) {
  .dashboard-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
@media (min-width: 1920px) {
  .overview,
  .dashboard-grid {
    gap: 18px;
  }
  .dashboard-grid {
    grid-template-columns: minmax(0, 1.45fr) minmax(460px, 0.9fr);
  }
}
</style>
