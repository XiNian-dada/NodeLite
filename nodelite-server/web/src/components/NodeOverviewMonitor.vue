<script setup lang="ts">
import { computed, reactive } from 'vue';
import { useI18n } from 'vue-i18n';
import type { HistoryPoint, NodeStatus } from '@/api';
import { provideChartHoverGroup } from '@/composables/useChartHoverGroup';
import { PRESET_WINDOWS, type PresetKey } from '@/composables/useChartSelection';
import {
  buildOverviewMonitorModel,
  type OverviewMonitorMetric,
} from '@/lib/nodeOverviewMonitorModel';
import MetricChart from './MetricChart.vue';

export type { OverviewMonitorMetric } from '@/lib/nodeOverviewMonitorModel';

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

const model = computed(() =>
  buildOverviewMonitorModel({
    node: props.node,
    history: props.history,
    clipSpikes,
    t,
  }),
);
</script>

<template>
  <div class="overview-monitor" data-test="node-combined-overview">
    <section class="info-band" data-test="overview-info-band">
      <div class="info-band__title">{{ t('node.info.title') }}</div>
      <div class="info-band__grid">
        <div v-for="row in model.infoRows" :key="row.label" class="info-pill">
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
        v-for="card in model.summaryCards"
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
      <article v-for="chart in model.charts" :key="chart.metric" class="chart-card">
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
