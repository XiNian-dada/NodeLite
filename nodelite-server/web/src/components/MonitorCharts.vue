<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { HistoryPoint, NodeStatus } from '@/api';
import { buildChartData } from '@/lib/chart/chartData';
import { provideChartHoverGroup } from '@/composables/useChartHoverGroup';
import { PRESET_WINDOWS, type PresetKey } from '@/composables/useChartSelection';
import MetricChart from './MetricChart.vue';

export type MonitorMetric = 'cpu' | 'memory' | 'network' | 'latency';

const props = defineProps<{
  node: NodeStatus | null;
  history: HistoryPoint[];
  activeKey: PresetKey;
}>();

const emit = defineEmits<{
  selectPreset: [key: PresetKey];
  zoom: [metric: MonitorMetric];
}>();

const { t } = useI18n();

// The four monitor charts link their crosshairs by timestamp.
provideChartHoverGroup();

const data = computed(() => buildChartData(props.history));

const networkSeries = computed(() => [
  { label: t('index.node.download'), color: 'var(--chart-network-down)', points: data.value.dlPts },
  { label: t('index.node.upload'), color: 'var(--chart-network-up)', points: data.value.upPts },
]);
</script>

<template>
  <div class="monitor" data-test="monitor-charts">
    <div class="monitor__presets" role="group" data-test="monitor-presets">
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

    <div class="monitor__grid">
      <article class="panel big-chart">
        <header class="big-chart__head">
          <span class="big-chart__title">{{ t('node.cpu_usage') }}</span>
          <button
            type="button"
            class="zoom-button"
            :aria-label="t('node.chart.zoom')"
            :title="t('node.chart.zoom')"
            data-test="zoom-cpu"
            @click="emit('zoom', 'cpu')"
          >
            ⤢
          </button>
        </header>
        <MetricChart :points="data.cpuPts" value-kind="percent" color="var(--chart-cpu)" :label="t('node.cpu_usage')" :min-value="0" :height="220" />
      </article>

      <article class="panel big-chart">
        <header class="big-chart__head">
          <span class="big-chart__title">{{ t('node.memory_usage') }}</span>
          <button type="button" class="zoom-button" :aria-label="t('node.chart.zoom')" :title="t('node.chart.zoom')" data-test="zoom-memory" @click="emit('zoom', 'memory')">⤢</button>
        </header>
        <MetricChart :points="data.memPts" value-kind="percent" color="var(--chart-memory)" :label="t('node.memory_usage')" :min-value="0" :height="220" />
      </article>

      <article class="panel big-chart">
        <header class="big-chart__head">
          <span class="big-chart__title">{{ t('node.network_traffic') }}</span>
          <button type="button" class="zoom-button" :aria-label="t('node.chart.zoom')" :title="t('node.chart.zoom')" data-test="zoom-network" @click="emit('zoom', 'network')">⤢</button>
        </header>
        <MetricChart :series="networkSeries" value-kind="rate" :min-value="0" :height="220" />
      </article>

      <article class="panel big-chart">
        <header class="big-chart__head">
          <span class="big-chart__title">{{ t('node.latency_history') }}</span>
          <button type="button" class="zoom-button" :aria-label="t('node.chart.zoom')" :title="t('node.chart.zoom')" data-test="zoom-latency" @click="emit('zoom', 'latency')">⤢</button>
        </header>
        <MetricChart :points="data.rttPts" value-kind="latency" color="var(--chart-latency)" :label="t('node.latency_history')" :min-value="0" :height="220" />
      </article>
    </div>
  </div>
</template>

<style scoped>
.monitor__presets {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 14px;
}
.preset-button {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 999px;
  color: var(--text-secondary);
  padding: 6px 14px;
  font-size: 12px;
}
.preset-button:hover {
  border-color: var(--border-strong);
}
.preset-button.active {
  color: var(--accent-blue);
  border-color: var(--accent-blue);
  background: var(--accent-blue-soft);
}
.monitor__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: 14px;
}
.big-chart {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 16px 18px;
}
.big-chart__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.big-chart__title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
}
.zoom-button {
  width: 28px;
  height: 28px;
  border-radius: 8px;
  border: 1px solid var(--border-soft);
  background: var(--bg-card-soft);
  color: var(--text-muted);
}
.zoom-button:hover {
  color: var(--text-primary);
}
</style>
