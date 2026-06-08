<script setup lang="ts">
import { computed, reactive } from 'vue';
import { useI18n } from 'vue-i18n';
import type { HistoryPoint, NodeStatus } from '@/api';
import { provideChartHoverGroup } from '@/composables/useChartHoverGroup';
import { PRESET_WINDOWS, type PresetKey } from '@/composables/useChartSelection';
import { buildChartData, averageValue, type ChartPoint } from '@/lib/chart/chartData';
import { formatChartValue, type ChartValueKind } from '@/lib/chart/format';
import { loadSeries, networkSeries } from '@/lib/chart/svgModel';
import { totalDiskBytes, uniqueDisks, usedDiskBytes } from '@/lib/disks';
import { fmtBytes, fmtRate, uptimeParts } from '@/lib/format';
import MetricChart from './MetricChart.vue';

export type OverviewMonitorMetric = 'cpu' | 'memory' | 'network' | 'load' | 'disk' | 'latency';

const props = defineProps<{
  node: NodeStatus | null;
  history: HistoryPoint[];
  activeKey: PresetKey;
}>();

const emit = defineEmits<{
  selectPreset: [key: PresetKey];
  zoom: [metric: OverviewMonitorMetric, clipSpikes: boolean];
}>();

const { t } = useI18n();

provideChartHoverGroup();

function currentHistoryPoint(node: NodeStatus | null): HistoryPoint | null {
  const snapshot = node?.snapshot;
  if (!node || !snapshot) return null;
  const disks = uniqueDisks(snapshot.disks);
  const totalDisk = totalDiskBytes(disks);
  const usedDisk = usedDiskBytes(disks);
  const memoryTotal = snapshot.memory.total_bytes;
  return {
    node_id: node.identity.node_id,
    recorded_at: snapshot.collected_at,
    cpu_usage_percent: snapshot.cpu_usage_percent,
    load_one: snapshot.load.one,
    load_five: snapshot.load.five,
    load_fifteen: snapshot.load.fifteen,
    memory_used_percent: memoryTotal > 0 ? (snapshot.memory.used_bytes / memoryTotal) * 100 : 0,
    rx_bytes_per_sec: snapshot.network.rx_bytes_per_sec,
    tx_bytes_per_sec: snapshot.network.tx_bytes_per_sec,
    latency_ms: node.latency_ms,
    packet_loss_percent: snapshot.network.packet_loss_percent,
    disk_used_percent: totalDisk > 0 ? (usedDisk / totalDisk) * 100 : null,
  };
}

const effectiveHistory = computed(() => {
  const current = currentHistoryPoint(props.node);
  if (!current) return props.history;
  if (props.history.some((point) => point.recorded_at === current.recorded_at)) {
    return props.history;
  }
  return [...props.history, current];
});

const data = computed(() => buildChartData(effectiveHistory.value));
const clipSpikes = reactive<Record<OverviewMonitorMetric, boolean>>({
  cpu: true,
  memory: true,
  network: true,
  load: true,
  disk: true,
  latency: true,
});

function toggleClip(metric: OverviewMonitorMetric): void {
  clipSpikes[metric] = !clipSpikes[metric];
}

function uptimeText(seconds: number | null | undefined): string {
  const parts = uptimeParts(seconds);
  if (!parts) return t('common.not_available');
  const named = { days: parts.days, hours: parts.hours, minutes: parts.minutes };
  if (parts.days > 0) return t('node.uptime.days_hours', named);
  if (parts.hours > 0) return t('node.uptime.hours_minutes', named);
  return t('node.uptime.minutes', named);
}

const infoRows = computed<Array<{ label: string; value: string }>>(() => {
  const node = props.node;
  if (!node) return [];
  const id = node.identity;
  const snapshot = node.snapshot;
  const disks = uniqueDisks(snapshot?.disks);
  const totalDisk = totalDiskBytes(disks);
  const cpuLine = id.cpu_cores
    ? `${t('node.info.cores', { count: id.cpu_cores })}${id.cpu_model ? ` · ${id.cpu_model}` : ''}`
    : (id.cpu_model ?? t('common.unknown'));

  return [
    { label: t('node.info.os'), value: id.os || t('common.unknown_os') },
    { label: t('node.info.kernel'), value: id.kernel_version || t('common.unknown') },
    { label: t('node.info.cpu'), value: cpuLine },
    {
      label: t('node.info.memory'),
      value: snapshot?.memory.total_bytes
        ? (fmtBytes(snapshot.memory.total_bytes) ?? t('common.not_available'))
        : t('common.not_available'),
    },
    {
      label: t('node.info.disk'),
      value: totalDisk
        ? (fmtBytes(totalDisk) ?? t('common.not_available'))
        : t('common.not_available'),
    },
    { label: t('node.info.virtualization'), value: id.agent_version || t('common.unknown') },
    { label: t('node.info.uptime'), value: uptimeText(snapshot?.uptime_secs) },
  ];
});

const memoryPercent = computed(() => {
  const memory = props.node?.snapshot?.memory;
  return memory?.total_bytes ? (memory.used_bytes / memory.total_bytes) * 100 : null;
});

const disk = computed(() => {
  const disks = uniqueDisks(props.node?.snapshot?.disks);
  const total = totalDiskBytes(disks);
  const used = usedDiskBytes(disks);
  const pct = total ? (used / total) * 100 : null;
  return {
    pct,
    used: total ? (fmtBytes(used) ?? '—') : '—',
    total: total ? (fmtBytes(total) ?? '—') : '—',
  };
});

function percentText(value: number | null | undefined): string {
  return value == null ? '—' : formatChartValue(value, 'percent');
}

function progress(value: number | null | undefined): number | null {
  if (value == null || !Number.isFinite(Number(value))) return null;
  return Math.max(0, Math.min(100, Number(value)));
}

function avgText(points: ChartPoint[], kind: ChartValueKind): string {
  const avg = averageValue(points);
  return t('node.chart.average', {
    value: avg == null ? '—' : formatChartValue(avg, kind),
  });
}

const cpuValue = computed(() => props.node?.snapshot?.cpu_usage_percent ?? null);
const loadValue = computed(() => props.node?.snapshot?.load ?? null);
const latencyValue = computed(() => props.node?.latency_ms ?? null);
const networkValue = computed(() => props.node?.snapshot?.network ?? null);

const summaryCards = computed(() => {
  const memory = props.node?.snapshot?.memory;
  return [
    {
      key: 'cpu',
      label: t('node.cpu_usage'),
      value: percentText(cpuValue.value),
      sub: avgText(data.value.cpuPts, 'percent'),
      progress: progress(cpuValue.value),
      tone: 'green',
    },
    {
      key: 'memory',
      label: t('node.memory_usage'),
      value: percentText(memoryPercent.value),
      sub: memory?.total_bytes
        ? `${fmtBytes(memory.used_bytes) ?? '—'} / ${fmtBytes(memory.total_bytes) ?? '—'}`
        : '—',
      progress: progress(memoryPercent.value),
      tone: 'blue',
    },
    {
      key: 'disk',
      label: t('node.disk_usage'),
      value: percentText(disk.value.pct),
      sub: `${disk.value.used} / ${disk.value.total}`,
      progress: progress(disk.value.pct),
      tone: 'teal',
    },
    {
      key: 'load',
      label: t('node.load'),
      value: loadValue.value ? loadValue.value.one.toFixed(2) : '—',
      sub: loadValue.value
        ? `${loadValue.value.one.toFixed(2)} / ${loadValue.value.five.toFixed(2)} / ${loadValue.value.fifteen.toFixed(2)}`
        : '—',
      progress: null,
      tone: 'neutral',
    },
    {
      key: 'latency',
      label: t('node.latency_history'),
      value: latencyValue.value == null ? '—' : formatChartValue(latencyValue.value, 'latency'),
      sub: avgText(data.value.rttPts, 'latency'),
      progress: null,
      tone: 'yellow',
    },
  ];
});

const charts = computed(() => [
  {
    metric: 'cpu' as const,
    title: t('node.cpu_usage'),
    meta: percentText(cpuValue.value),
    sub: avgText(data.value.cpuPts, 'percent'),
    chartProps: {
      points: data.value.cpuPts,
      valueKind: 'percent' as const,
      color: 'var(--chart-cpu)',
      label: t('node.cpu_usage'),
      clipSpikes: clipSpikes.cpu,
    },
  },
  {
    metric: 'memory' as const,
    title: t('node.memory_usage'),
    meta: percentText(memoryPercent.value),
    sub: avgText(data.value.memPts, 'percent'),
    chartProps: {
      points: data.value.memPts,
      valueKind: 'percent' as const,
      color: 'var(--chart-memory)',
      label: t('node.memory_usage'),
      maxValue: 100,
      clipSpikes: clipSpikes.memory,
    },
  },
  {
    metric: 'network' as const,
    title: t('node.network_traffic'),
    meta: fmtRate(networkValue.value?.rx_bytes_per_sec) ?? '—',
    sub: fmtRate(networkValue.value?.tx_bytes_per_sec) ?? '—',
    chartProps: {
      series: networkSeries(data.value, t('index.node.download'), t('index.node.upload')),
      valueKind: 'rate' as const,
      clipSpikes: clipSpikes.network,
    },
  },
  {
    metric: 'load' as const,
    title: t('node.load'),
    meta: loadValue.value ? loadValue.value.one.toFixed(2) : '—',
    sub: avgText(data.value.loadOnePts, 'number'),
    chartProps: {
      series: loadSeries(data.value),
      valueKind: 'number' as const,
      clipSpikes: clipSpikes.load,
    },
  },
  {
    metric: 'disk' as const,
    title: t('node.disk_usage'),
    meta: percentText(disk.value.pct),
    sub: avgText(data.value.diskPts, 'percent'),
    chartProps: {
      points: data.value.diskPts,
      valueKind: 'percent' as const,
      color: 'var(--chart-disk)',
      label: t('node.disk_usage'),
      maxValue: 100,
      clipSpikes: clipSpikes.disk,
    },
  },
  {
    metric: 'latency' as const,
    title: t('node.latency_history'),
    meta: latencyValue.value == null ? '—' : formatChartValue(latencyValue.value, 'latency'),
    sub: avgText(data.value.rttPts, 'latency'),
    chartProps: {
      points: data.value.rttPts,
      valueKind: 'latency' as const,
      color: 'var(--chart-latency)',
      label: t('node.latency_history'),
      clipSpikes: clipSpikes.latency,
    },
  },
]);
</script>

<template>
  <div class="overview-monitor" data-test="node-combined-overview">
    <section class="info-band" data-test="overview-info-band">
      <div class="info-band__title">{{ t('node.info.title') }}</div>
      <div class="info-band__grid">
        <div v-for="row in infoRows" :key="row.label" class="info-pill">
          <span class="info-pill__label">{{ row.label }}</span>
          <strong class="info-pill__value">{{ row.value }}</strong>
        </div>
      </div>
    </section>

    <div class="window-row">
      <span class="window-row__label">{{ t('node.history_window') }}</span>
      <div class="preset-segment" role="group" data-test="monitor-presets">
        <button
          v-for="preset in PRESET_WINDOWS"
          :key="preset.key"
          type="button"
          class="preset-button"
          :class="{ active: activeKey === preset.key }"
          :data-test="`preset-${preset.key}`"
          @click="emit('selectPreset', preset.key)"
        >
          {{ t(`node.preset.${preset.key}`) }}
        </button>
      </div>
    </div>

    <div class="summary-grid" data-test="overview-summary-cards">
      <article
        v-for="card in summaryCards"
        :key="card.key"
        class="summary-card"
        :class="`summary-card--${card.tone}`"
        :data-test="`summary-${card.key}`"
      >
        <span class="summary-card__label">{{ card.label }}</span>
        <strong class="summary-card__value">{{ card.value }}</strong>
        <div v-if="card.progress != null" class="summary-card__bar">
          <span :style="{ width: `${card.progress}%` }" />
        </div>
        <small class="summary-card__sub">{{ card.sub }}</small>
      </article>
    </div>

    <div class="chart-grid" data-test="overview-monitor-charts">
      <article v-for="chart in charts" :key="chart.metric" class="chart-card">
        <header class="chart-card__head">
          <div class="chart-card__title-wrap">
            <span class="chart-card__title">{{ chart.title }}</span>
            <span class="chart-card__meta">
              <strong>{{ chart.meta }}</strong>
              <small>{{ chart.sub }}</small>
            </span>
          </div>
          <div class="chart-card__actions">
            <button
              type="button"
              class="clip-toggle"
              :class="{ active: clipSpikes[chart.metric] }"
              :aria-label="clipSpikes[chart.metric] ? t('node.clip.on') : t('node.clip.off')"
              :aria-pressed="clipSpikes[chart.metric]"
              :title="clipSpikes[chart.metric] ? t('node.clip.on') : t('node.clip.off')"
              :data-test="`clip-${chart.metric}`"
              @click="toggleClip(chart.metric)"
            >
              <span class="clip-toggle__knob" />
            </button>
            <button
              type="button"
              class="zoom-button"
              :aria-label="t('node.chart.zoom')"
              :title="t('node.chart.zoom')"
              :data-test="`zoom-${chart.metric}`"
              @click="emit('zoom', chart.metric, clipSpikes[chart.metric])"
            >
              ⤢
            </button>
          </div>
        </header>
        <MetricChart v-bind="chart.chartProps" :min-value="0" :height="220" />
      </article>
    </div>
  </div>
</template>

<style scoped>
.overview-monitor {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.info-band {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 16px;
}

.info-band__title {
  color: var(--text-secondary);
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 12px;
}

.info-band__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 10px;
}

.info-pill {
  min-width: 0;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  padding: 10px 12px;
}

.info-pill__label,
.summary-card__label,
.chart-card__meta small,
.window-row__label {
  color: var(--text-muted);
  font-size: 12px;
}

.info-pill__value {
  display: block;
  overflow-wrap: anywhere;
  color: var(--text-primary);
  font-size: 13px;
  font-weight: 500;
  line-height: 1.35;
  margin-top: 4px;
}

.window-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.preset-segment {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card);
  padding: 3px;
}

.preset-button {
  box-sizing: border-box;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  min-width: 56px;
  height: 30px;
  padding: 0 10px;
  font-size: 12px;
  white-space: nowrap;
}

.preset-button:hover {
  color: var(--text-secondary);
  background: var(--bg-card-soft);
}

.preset-button.active {
  color: var(--bg-app);
  background: var(--text-primary);
}

.summary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: 12px;
}

.summary-card {
  min-height: 122px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 8px;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card);
  padding: 14px;
}

.summary-card__value {
  color: var(--text-primary);
  font-size: 26px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  letter-spacing: 0;
}

.summary-card__bar {
  height: 5px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--bg-card-soft);
}

.summary-card__bar span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: currentColor;
}

.summary-card--green {
  color: var(--chart-cpu);
}

.summary-card--blue {
  color: var(--chart-memory);
}

.summary-card--teal {
  color: var(--chart-disk);
}

.summary-card--yellow {
  color: var(--chart-latency);
}

.summary-card--neutral {
  color: var(--text-secondary);
}

.summary-card__sub {
  color: var(--text-muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.chart-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr));
  gap: 12px;
}

.chart-card {
  min-width: 0;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card);
  padding: 14px 14px 12px;
}

.chart-card__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  min-height: 42px;
  margin-bottom: 10px;
}

.chart-card__title-wrap,
.chart-card__meta {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.chart-card__title {
  color: var(--text-secondary);
  font-size: 13px;
  font-weight: 600;
}

.chart-card__meta {
  gap: 2px;
  margin-top: 4px;
}

.chart-card__meta strong {
  color: var(--text-primary);
  font-size: 16px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
}

.chart-card__actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 auto;
}

.clip-toggle,
.zoom-button {
  height: 28px;
  border: 1px solid var(--border-soft);
  border-radius: 6px;
  background: var(--bg-card-soft);
  color: var(--text-muted);
  font-size: 11px;
}

.clip-toggle {
  position: relative;
  width: 36px;
  padding: 0;
}

.clip-toggle__knob {
  position: absolute;
  top: 50%;
  left: 5px;
  width: 12px;
  height: 12px;
  border-radius: 999px;
  background: currentColor;
  transform: translateY(-50%);
  transition: left 150ms ease;
}

.clip-toggle.active {
  color: var(--bg-app);
  background: var(--text-secondary);
  border-color: var(--text-secondary);
}

.clip-toggle.active .clip-toggle__knob {
  left: 18px;
}

.zoom-button {
  width: 30px;
  padding: 0;
  font-size: 12px;
}

.clip-toggle:hover,
.zoom-button:hover {
  color: var(--text-primary);
  border-color: var(--border-strong);
}

.clip-toggle.active:hover {
  color: var(--bg-app);
}

@media (min-width: 1680px) {
  .chart-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

@media (max-width: 700px) {
  .preset-segment {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    width: 100%;
  }

  .preset-button {
    min-width: 0;
    padding: 0 4px;
    font-size: 11px;
  }
}
</style>
