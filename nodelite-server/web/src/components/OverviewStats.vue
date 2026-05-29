<script setup lang="ts">
import { computed } from 'vue';
import { useOverviewStore } from '@/stores/overview';

const store = useOverviewStore();

const PLACEHOLDER = '--';

const total = computed(() =>
  store.data ? String(store.data.total_nodes) : PLACEHOLDER,
);
const online = computed(() =>
  store.data ? String(store.data.online_nodes) : PLACEHOLDER,
);
const offline = computed(() =>
  store.data ? String(store.data.offline_nodes) : PLACEHOLDER,
);
const latency = computed(() => {
  const ms = store.data?.average_latency_ms;
  return ms === null || ms === undefined ? PLACEHOLDER : ms.toFixed(0);
});
</script>

<template>
  <div class="stats-grid" data-test="overview-stats">
    <article class="stat-card">
      <div class="label">{{ $t('index.stat.total') }}</div>
      <div class="row">
        <div class="value" data-test="stat-total">{{ total }}</div>
      </div>
    </article>

    <article class="stat-card online">
      <div class="label">{{ $t('index.stat.online') }}</div>
      <div class="row">
        <div class="value" data-test="stat-online">{{ online }}</div>
      </div>
    </article>

    <article class="stat-card offline">
      <div class="label">{{ $t('index.stat.offline') }}</div>
      <div class="row">
        <div class="value" data-test="stat-offline">{{ offline }}</div>
      </div>
    </article>

    <article class="stat-card latency">
      <div class="label">{{ $t('index.stat.latency') }}</div>
      <div class="row">
        <div class="value" data-test="stat-latency">
          {{ latency }}<small class="value-unit"> ms</small>
        </div>
      </div>
    </article>
  </div>
</template>

<style scoped>
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
}
.stat-card {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 16px 18px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 110px;
}
.stat-card .label {
  color: var(--text-muted);
  font-size: 13px;
}
.stat-card .row {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 10px;
}
.stat-card .value {
  font-size: 28px;
  font-weight: 600;
  letter-spacing: -0.02em;
  line-height: 1;
}
.stat-card.online .value {
  color: var(--accent-green);
}
.stat-card.offline .value {
  color: var(--accent-red);
}
.stat-card.latency .value {
  color: var(--accent-blue);
}
.stat-card .value-unit {
  font-size: 14px;
  color: var(--text-muted);
  font-weight: 500;
  margin-left: 4px;
}
</style>
