<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { storeToRefs } from 'pinia';
import type { NodeListItem } from '@/api';
import { nodeFlag, nodeStatusKey } from '@/lib/map/projection';
import { buildSparkline, nodeSparkPoints, sparklineColor } from '@/lib/chart/sparkline';
import { fmtBytes } from '@/lib/format';
import { locationFromNode } from '@/lib/nodeMeta';
import { useNodeHistoryStore } from '@/stores/nodeHistory';
import { useSettingsStore } from '@/stores/settings';

const props = defineProps<{ node: NodeListItem }>();

const historyStore = useNodeHistoryStore();
const settingsStore = useSettingsStore();
const { entries: historyEntries } = storeToRefs(historyStore);
const { t, locale } = useI18n();
const liveLoadPoints = ref<number[]>([]);
const LIVE_SPARK_MAX_POINTS = 36;

const nodeId = computed(() => props.node.identity.node_id);
const status = computed(() => nodeStatusKey(props.node));

const title = computed(() => {
  const { node_label: label, node_id: id } = props.node.identity;
  return label && label !== id ? `${label}: ${id}` : id;
});

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

const latencyText = computed(() =>
  props.node.latency_ms == null ? '—' : `${Math.round(props.node.latency_ms)} ms`,
);

const locationText = computed(() => {
  return locationFromNode(props.node)?.replace(', ', ' / ') || props.node.identity.hostname;
});

const loadText = computed(() => {
  const one = props.node.snapshot?.load.one;
  return one == null ? '—' : one.toFixed(2);
});

const cpu = computed(() => props.node.snapshot?.cpu_usage_percent ?? null);
const cpuText = computed(() => (cpu.value == null ? '—' : `${cpu.value.toFixed(0)}%`));
const cpuFill = computed(() => clampPercent(cpu.value));
const cpuClass = computed(() => {
  const v = cpu.value;
  if (v == null) return '';
  if (v >= 80) return 'accent-red';
  if (v >= 50) return 'accent-yellow';
  return 'accent-green';
});

const memory = computed(() => {
  const value = props.node.snapshot?.memory;
  if (!value) {
    return {
      pctText: '—',
      usedText: '—',
      totalText: '—',
      fill: 0,
      tone: '',
    };
  }
  const pct = (value.used_bytes / Math.max(value.total_bytes, 1)) * 100;
  return {
    pctText: `${pct.toFixed(0)}%`,
    usedText: fmtBytes(value.used_bytes) ?? '—',
    totalText: fmtBytes(value.total_bytes) ?? '—',
    fill: clampPercent(pct),
    tone: pct >= 85 ? 'accent-red' : pct >= 70 ? 'accent-yellow' : 'accent-green',
  };
});

const sparkColor = computed(() => sparklineColor(status.value));
const historyPoints = computed(() => historyEntries.value[nodeId.value]?.points ?? []);
const historySparkPoints = computed(() => nodeSparkPoints(historyPoints.value));
const sparkPoints = computed(() => {
  const current = props.node.snapshot?.load.one;
  const currentValue = Number(current);
  const hasCurrent = Number.isFinite(currentValue);
  const history = historySparkPoints.value;
  if (history.length >= 2) {
    if (hasCurrent && history[history.length - 1] !== currentValue) {
      return [...history, currentValue];
    }
    return history;
  }
  if (history.length === 1 && hasCurrent && history[0] !== currentValue) {
    return [...history, currentValue];
  }
  if (liveLoadPoints.value.length >= 2) return liveLoadPoints.value;
  return nodeSparkPoints(historyPoints.value, current);
});
const spark = computed(() => buildSparkline(sparkPoints.value));
const serviceMeta = computed(() =>
  settingsStore.data?.agents.find((agent) => agent.node_id === nodeId.value),
);
const serviceExpiryText = computed(() => {
  const meta = serviceMeta.value;
  if (!meta) return '—';
  if (meta.service_unlimited) return t('index.node.service_unlimited');
  if (!meta.service_expires_at) return '—';
  const ms = Date.parse(meta.service_expires_at);
  return Number.isFinite(ms)
    ? new Date(ms).toLocaleDateString(locale.value, {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
      })
    : meta.service_expires_at;
});
const renewalPriceText = computed(() => {
  const meta = serviceMeta.value;
  if (!meta) return '—';
  return meta.renewal_price || (meta.service_unlimited ? t('index.node.self_owned') : '—');
});

function clampPercent(value: number | null): number {
  if (value == null || !Number.isFinite(value)) return 0;
  return Math.max(2, Math.min(100, value));
}

function recordLiveLoad(value: number | null | undefined): void {
  if (value == null || !Number.isFinite(Number(value))) return;
  liveLoadPoints.value = [...liveLoadPoints.value, Number(value)].slice(-LIVE_SPARK_MAX_POINTS);
}

// Re-request on every snapshot change (the 5s poll replaces node objects),
// throttled to once a minute by the store's TTL. NodeCard is keyed by
// node_id so the instance is reused across polls — onMounted alone would
// fire only once and freeze the sparkline.
watch(
  () => props.node.snapshot,
  (snapshot) => {
    recordLiveLoad(snapshot?.load.one);
    void historyStore.loadIfStale(nodeId.value);
  },
  {
    immediate: true,
  },
);
</script>

<template>
  <RouterLink
    class="node-card"
    :class="status"
    :to="`/nodes/${encodeURIComponent(nodeId)}`"
    data-test="node-card"
    :data-node-id="nodeId"
  >
    <div class="node-card-head">
      <div class="node-card-title-block">
        <span class="flag">{{ nodeFlag(node) }}</span>
        <span class="title-copy">
          <span class="node-card-title" :title="title">{{ title }}</span>
          <span class="node-card-meta" :title="locationText">{{ locationText }}</span>
        </span>
      </div>
      <span class="badge" :class="status" data-test="node-badge">
        {{ $t(statusLabelKey) }}
      </span>
    </div>

    <div class="resource-list">
      <div class="resource-row">
        <div class="resource-label">
          <span>{{ $t('index.node.cpu') }}</span>
          <strong :class="cpuClass" data-test="metric-cpu">{{ cpuText }}</strong>
        </div>
        <div class="meter">
          <span :class="cpuClass" :style="{ width: `${cpuFill}%` }" />
        </div>
      </div>
      <div class="resource-row">
        <div class="resource-label">
          <span>{{ $t('index.node.memory') }}</span>
          <strong :class="memory.tone" data-test="metric-memory">{{ memory.pctText }}</strong>
        </div>
        <div class="meter">
          <span :class="memory.tone" :style="{ width: `${memory.fill}%` }" />
        </div>
      </div>
    </div>

    <div class="node-metrics">
      <div class="node-metric">
        <div class="label">{{ $t('index.node.load') }}</div>
        <div class="value" data-test="metric-load">{{ loadText }}</div>
      </div>
      <div class="node-metric">
        <div class="label">{{ $t('index.node.memory_used') }}</div>
        <div class="value compact">{{ memory.usedText }} / {{ memory.totalText }}</div>
      </div>
      <div class="node-metric">
        <div class="label">{{ $t('index.node.latency') }}</div>
        <div class="value" data-test="metric-latency">{{ latencyText }}</div>
      </div>
    </div>

    <div class="service-row">
      <div class="service-item">
        <span>{{ $t('index.node.service_expiry') }}</span>
        <strong data-test="node-service-expiry">{{ serviceExpiryText }}</strong>
      </div>
      <div class="service-item right">
        <span>{{ $t('index.node.renewal_price') }}</span>
        <strong data-test="node-renewal-price">{{ renewalPriceText }}</strong>
      </div>
    </div>

    <div class="node-spark" :style="{ color: sparkColor }">
      <svg
        v-if="spark"
        :viewBox="`0 0 ${spark.width} ${spark.height}`"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <path :d="spark.area" :fill="sparkColor" fill-opacity="0.16" />
        <path
          :d="spark.line"
          fill="none"
          :stroke="sparkColor"
          stroke-width="1.1"
          stroke-linecap="round"
          stroke-linejoin="round"
          vector-effect="non-scaling-stroke"
        />
      </svg>
      <svg v-else viewBox="0 0 200 60" preserveAspectRatio="none" aria-hidden="true">
        <line
          x1="0"
          y1="48"
          x2="200"
          y2="48"
          :stroke="sparkColor"
          stroke-width="1"
          stroke-opacity="0.52"
        />
      </svg>
    </div>
  </RouterLink>
</template>

<style scoped>
.node-card {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  box-shadow: var(--panel-shadow);
  padding: 16px 16px 0;
  display: flex;
  flex-direction: column;
  min-height: var(--node-card-height, 370px);
  transition:
    transform 160ms ease,
    border-color 160ms ease;
  overflow: hidden;
}
.node-card:hover {
  border-color: var(--border-strong);
  transform: translateY(-1px);
}
.node-card.online:hover {
  border-color: rgba(37, 228, 135, 0.34);
}
.node-card.latency:hover {
  border-color: rgba(245, 197, 66, 0.34);
}
.node-card.offline:hover {
  border-color: rgba(255, 77, 109, 0.34);
}
.node-card-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 10px;
}
.node-card-title-block {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  min-width: 0;
}
.title-copy {
  display: grid;
  gap: 2px;
  min-width: 0;
}
.node-card-title {
  font-weight: 600;
  font-size: 15px;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-card-title-block .flag {
  font-size: 18px;
  line-height: 1;
}
.node-card-meta {
  color: var(--text-muted);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
  white-space: nowrap;
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
.node-metrics {
  flex-shrink: 0;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  margin: 14px 0 6px;
  overflow: hidden;
}
.node-metric {
  min-width: 0;
  padding: 10px 12px;
}
.node-metric + .node-metric {
  border-left: 1px solid var(--border-soft);
}
.node-metric .label,
.resource-label span {
  font-size: 11px;
  color: var(--text-muted);
  margin-bottom: 2px;
}
.node-metric .value,
.resource-label strong {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}
.node-metric .value.compact {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-metric .value.accent-green,
.resource-label strong.accent-green,
.meter span.accent-green {
  color: var(--accent-green);
}
.node-metric .value.accent-yellow,
.resource-label strong.accent-yellow,
.meter span.accent-yellow {
  color: var(--accent-yellow);
}
.node-metric .value.accent-red,
.resource-label strong.accent-red,
.meter span.accent-red {
  color: var(--accent-red);
}
.resource-list {
  display: grid;
  gap: 12px;
  margin-top: 18px;
}
.resource-label {
  align-items: center;
  display: flex;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 6px;
}
.meter {
  background: var(--bg-card-soft);
  border-radius: 999px;
  height: 7px;
  overflow: hidden;
}
.meter span {
  background: currentColor;
  border-radius: inherit;
  display: block;
  height: 100%;
  min-width: 0;
}
.service-row {
  display: flex;
  justify-content: space-between;
  gap: 14px;
  margin-top: auto;
  padding-top: 12px;
}
.service-item {
  min-width: 0;
  display: grid;
  gap: 2px;
}
.service-item.right {
  text-align: right;
}
.service-item span {
  color: var(--text-muted);
  font-size: 11px;
}
.service-item strong {
  color: var(--text-primary);
  font-size: 12px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-spark {
  flex-shrink: 0;
  height: 58px;
  margin: 8px -16px 0;
  position: relative;
}
.node-spark svg {
  width: 100%;
  height: 100%;
  display: block;
}
</style>
