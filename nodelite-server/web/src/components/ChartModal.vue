<script setup lang="ts">
import type { ChartPoint } from '@/lib/chart/chartData';
import type { ChartValueKind } from '@/lib/chart/format';
import type { MultiSeriesInput } from '@/lib/chart/svgModel';
import MetricChart from './MetricChart.vue';

defineProps<{
  open: boolean;
  title: string;
  points?: ChartPoint[];
  series?: MultiSeriesInput[];
  valueKind: ChartValueKind;
  color?: string;
}>();

const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <div
    v-if="open"
    class="chart-modal"
    data-test="chart-modal"
    role="dialog"
    aria-modal="true"
    @click.self="emit('close')"
  >
    <div class="chart-modal__panel">
      <header class="chart-modal__head">
        <h2 class="chart-modal__title">{{ title }}</h2>
        <button type="button" class="chart-modal__close" data-test="chart-modal-close" @click="emit('close')">
          ✕
        </button>
      </header>
      <MetricChart
        :points="points"
        :series="series"
        :value-kind="valueKind"
        :color="color ?? 'var(--accent-blue)'"
        :label="title"
        :min-value="0"
        :height="360"
      />
    </div>
  </div>
</template>

<style scoped>
.chart-modal {
  position: fixed;
  inset: 0;
  background: rgba(2, 6, 16, 0.62);
  display: grid;
  place-items: center;
  padding: 24px;
  z-index: 50;
}
.chart-modal__panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 18px;
  padding: 20px 22px;
  width: min(960px, 100%);
}
.chart-modal__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
}
.chart-modal__title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.chart-modal__close {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  border: 1px solid var(--border-soft);
  background: var(--bg-card-soft);
  color: var(--text-muted);
}
.chart-modal__close:hover {
  color: var(--text-primary);
}
</style>
