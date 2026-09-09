<script setup lang="ts">
import { computed, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import type { ChartPoint } from '@/lib/chart/chartData';
import type { ChartValueKind } from '@/lib/chart/format';
import type { MultiSeriesInput } from '@/lib/chart/svgModel';
import MetricChart from './MetricChart.vue';
import NativeDialog from './NativeDialog.vue';

// Visibility is owned by the parent via v-if (it already gates on having a
// selected metric), so there's no `open` prop here — the modal renders its
// content whenever mounted.
const props = defineProps<{
  title: string;
  points?: ChartPoint[];
  series?: MultiSeriesInput[];
  valueKind: ChartValueKind;
  color?: string;
  maxValue?: number;
  clipSpikes?: boolean;
}>();

const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const titleId = useId();

// Only include points OR series — exactOptionalPropertyTypes forbids passing
// an explicit undefined to an optional prop, so omit the absent one.
const chartProps = computed(() => ({
  valueKind: props.valueKind,
  color: props.color ?? 'var(--accent-blue)',
  label: props.title,
  minValue: 0,
  height: 260,
  clipSpikes: props.clipSpikes ?? true,
  ...(props.maxValue !== undefined ? { maxValue: props.maxValue } : {}),
  ...(props.points ? { points: props.points } : {}),
  ...(props.series ? { series: props.series } : {}),
}));
</script>

<template>
  <NativeDialog
    class="chart-modal"
    data-test="chart-modal"
    :labelled-by="titleId"
    @close="emit('close')"
  >
    <div class="chart-modal__panel">
      <header class="chart-modal__head">
        <h2 :id="titleId" class="chart-modal__title">{{ title }}</h2>
        <button
          type="button"
          class="chart-modal__close"
          data-test="chart-modal-close"
          :aria-label="t('common.close')"
          autofocus
          @click="emit('close')"
        >
          ✕
        </button>
      </header>
      <MetricChart v-bind="chartProps" />
    </div>
  </NativeDialog>
</template>

<style scoped>
.chart-modal {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.72);
  display: grid;
  place-items: center;
  padding: 24px;
  z-index: 50;
}
.chart-modal__panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 12px;
  padding: 16px 18px 18px;
  width: min(820px, 100%);
  max-height: calc(100vh - 48px);
  overflow: auto;
}
.chart-modal__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
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
