<script setup lang="ts">
import { computed, onMounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import AppLayout from '@/components/AppLayout.vue';
import { usePolling } from '@/composables/usePolling';
import { nodeStatusKey } from '@/lib/map/projection';
import { ipFromNode, locationFromNode } from '@/lib/nodeMeta';
import { uptimeParts } from '@/lib/format';
import { useNodeStatusStore } from '@/stores/nodeStatus';

const NODE_DETAIL_REFRESH_MS = 5000;

// Tabs the shell renders. `settings` is deferred (Stage 2.5) and rendered
// disabled, mirroring the dashboard sidebar pattern.
const TABS = ['overview', 'monitor', 'network', 'hardware', 'logs'] as const;
type TabId = (typeof TABS)[number];

function isTabId(value: string): value is TabId {
  return (TABS as readonly string[]).includes(value);
}

const route = useRoute();
const router = useRouter();
const store = useNodeStatusStore();

const nodeId = computed(() => String(route.params.id ?? ''));
const node = computed(() => store.data);

// Active tab is driven by the URL hash (e.g. /nodes/x#monitor), matching the
// legacy hash sync; falls back to overview.
const activeTab = computed<TabId>(() => {
  const hash = route.hash.replace(/^#/, '');
  return isTabId(hash) ? hash : 'overview';
});

function selectTab(tab: TabId): void {
  void router.replace({ hash: `#${tab}` });
}

const status = computed(() => (node.value ? nodeStatusKey(node.value) : 'offline'));
const statusLabelKey = computed(() => {
  switch (status.value) {
    case 'offline':
      return 'common.offline';
    case 'latency':
      return 'common.latency_warn';
    default:
      return 'common.online';
  }
});

const title = computed(
  () => node.value?.identity.node_label || node.value?.identity.node_id || nodeId.value,
);
const ip = computed(() => (node.value ? ipFromNode(node.value) : null));
const location = computed(() => (node.value ? locationFromNode(node.value) : null));
const uptime = computed(() => uptimeParts(node.value?.snapshot?.uptime_secs));

onMounted(() => {
  void store.load(nodeId.value);
});

// Navigating between nodes (same component, new :id) reloads.
watch(nodeId, (id) => {
  if (id) void store.load(id);
});

usePolling(() => store.refresh(), NODE_DETAIL_REFRESH_MS);
</script>

<template>
  <AppLayout>
    <template #title>
      <div class="node-title" data-test="node-detail-view">
        <h1 class="node-title__name">{{ title }}</h1>
        <span class="badge" :class="status" data-test="node-status-badge">
          {{ $t(statusLabelKey) }}
        </span>
        <div class="node-title__meta" data-test="node-meta">
          <span v-if="ip">{{ $t('node.meta.ip', { ip }) }}</span>
          <span v-if="location">{{ location }}</span>
          <span v-if="uptime && uptime.days > 0">{{ $t('node.meta.uptime_days', { days: uptime.days }) }}</span>
          <span v-else-if="uptime">{{ $t('node.meta.uptime_hours', { hours: uptime.hours }) }}</span>
        </div>
      </div>
    </template>

    <div class="node-detail">
      <nav class="tabs" data-test="node-tabs">
        <button
          v-for="tab in TABS"
          :key="tab"
          type="button"
          class="tab-button"
          :class="{ active: activeTab === tab }"
          :data-test="`tab-${tab}`"
          @click="selectTab(tab)"
        >
          {{ $t(`node.tabs.${tab}`) }}
        </button>
        <button
          type="button"
          class="tab-button"
          disabled
          :title="`${$t('node.tabs.settings')} (Stage 2.5)`"
          data-test="tab-settings"
        >
          {{ $t('node.tabs.settings') }}
        </button>
      </nav>

      <section class="tab-pane" :data-pane="activeTab" data-test="node-tab-pane">
        <p class="placeholder">{{ activeTab }} — coming in Stage 3c/3d</p>
      </section>
    </div>
  </AppLayout>
</template>

<style scoped>
.node-title {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}
.node-title__name {
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  letter-spacing: -0.01em;
}
.node-title__meta {
  display: flex;
  gap: 12px;
  color: var(--text-muted);
  font-size: 13px;
  width: 100%;
}
.badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 500;
  padding: 4px 8px;
  border-radius: 999px;
  background: var(--bg-card-soft);
  color: var(--text-muted);
}
.badge::before {
  content: '';
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.badge.online {
  color: var(--accent-green);
  background: var(--accent-green-soft);
}
.badge.latency {
  color: var(--accent-yellow);
  background: var(--accent-yellow-soft);
}
.badge.offline {
  color: var(--accent-red);
  background: var(--accent-red-soft);
}
.tabs {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
  border-bottom: 1px solid var(--border-soft);
  margin-bottom: 18px;
}
.tab-button {
  background: transparent;
  border: 0;
  border-bottom: 2px solid transparent;
  color: var(--text-muted);
  padding: 8px 14px;
  font-size: 13px;
  font-weight: 500;
}
.tab-button:hover:not([disabled]) {
  color: var(--text-secondary);
}
.tab-button.active {
  color: var(--accent-blue);
  border-bottom-color: var(--accent-blue);
}
.tab-button[disabled] {
  opacity: 0.4;
  cursor: not-allowed;
}
.placeholder {
  color: var(--text-muted);
  font-size: 13px;
}
</style>
