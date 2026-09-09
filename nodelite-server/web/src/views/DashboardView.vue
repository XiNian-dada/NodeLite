<script setup lang="ts">
import { computed, onMounted } from 'vue';
import AppLayout from '@/components/AppLayout.vue';
import OverviewStats from '@/components/OverviewStats.vue';
import NodeHealthMatrix from '@/components/NodeHealthMatrix.vue';
import NodeMap from '@/components/NodeMap.vue';
import NodeList from '@/components/NodeList.vue';
import LoginNotification from '@/components/LoginNotification.vue';
import { useBootstrapStore } from '@/stores/bootstrap';
import { useOverviewStore } from '@/stores/overview';
import { useSettingsStore } from '@/stores/settings';

const bootstrapStore = useBootstrapStore();
const overviewStore = useOverviewStore();
const settingsStore = useSettingsStore();

const onlineCount = computed(() => overviewStore.data?.online_nodes ?? 0);

onMounted(() => {
  void bootstrapStore.load();
  void settingsStore.load();
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

    <LoginNotification />
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
