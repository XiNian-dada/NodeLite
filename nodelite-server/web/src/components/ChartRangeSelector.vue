<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { HistoryPoint } from '@/api';
import { smoothPath } from '@/lib/chart/geometry';

export interface TimeRange {
  start: number;
  end: number;
}

const props = defineProps<{ history: HistoryPoint[]; modelValue: TimeRange | null }>();
const emit = defineEmits<{ 'update:modelValue': [value: TimeRange | null] }>();
const { t, locale } = useI18n({
  useScope: 'local',
  messages: {
    en: {
      range: 'Time range',
      start: 'Start',
      end: 'End',
      live: 'Latest hour',
      day: 'Past 24 hours',
      sixHours: 'Past 6 hours',
      all: 'All available history',
      context: 'Navigator window',
      hint: 'Drag either handle to zoom. Release to load detailed samples.',
      empty: 'Waiting for history',
    },
    'zh-CN': {
      range: '时间范围',
      start: '开始',
      end: '结束',
      live: '最近 1 小时',
      day: '最近 24 小时',
      sixHours: '最近 6 小时',
      all: '全部可用历史',
      context: '缩略图范围',
      hint: '拖动两端选择区间，松开后加载详细数据。',
      empty: '等待历史数据',
    },
  },
});
const draft = ref<TimeRange | null>(null);
const contextHours = ref(6);
watch(
  () => props.modelValue,
  () => {
    draft.value = null;
  },
);
const allSamples = computed(() =>
  props.history
    .map((point) => ({
      time: Math.floor(Date.parse(point.recorded_at) / 1000),
      value: Math.max(0, (point.rx_bytes_per_sec ?? 0) + (point.tx_bytes_per_sec ?? 0)),
    }))
    .filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value))
    .sort((a, b) => a.time - b.time),
);
const samples = computed(() => {
  const end = allSamples.value.at(-1)?.time ?? 0;
  return allSamples.value.filter((point) => point.time >= end - contextHours.value * 3600);
});
const bounds = computed(() => {
  const end = samples.value.at(-1)?.time ?? 0;
  return {
    start: Math.min(samples.value[0]?.time ?? 0, end - 3600, props.modelValue?.start ?? end),
    end: Math.max(end, props.modelValue?.end ?? end),
  };
});
const range = computed(() => {
  const { start: min, end: max } = bounds.value;
  const value = draft.value ?? props.modelValue ?? { start: max - 3600, end: max };
  const start = Math.max(min, Math.min(value.start, max - 60));
  return { start, end: Math.max(start + 60, Math.min(value.end, max)) };
});
const selectionStyle = computed(() => {
  const span = bounds.value.end - bounds.value.start;
  const start = (range.value.start - bounds.value.start) / span;
  const width = (range.value.end - range.value.start) / span;
  return {
    left: `calc(${100 * start}% + ${8 - 16 * start}px)`,
    width: `calc(${100 * width}% - ${16 * width}px)`,
  };
});
const preview = computed(() => {
  const max = Math.max(1, ...samples.value.map((point) => point.value));
  const span = bounds.value.end - bounds.value.start;
  return smoothPath(
    samples.value.map((point) => [
      (1000 * (point.time - bounds.value.start)) / span,
      54 - (point.value / max) * 46,
    ]),
  );
});
function describeTime(value: number): string {
  return new Date(value * 1000).toLocaleString(locale.value, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}
function inputTime(value: number): string {
  const date = new Date(value * 1000);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
}
function update(edge: 'start' | 'end', value: number): void {
  if (!Number.isFinite(value)) return;
  draft.value =
    edge === 'start'
      ? {
          start: Math.max(bounds.value.start, Math.min(value, range.value.end - 60)),
          end: range.value.end,
        }
      : {
          start: range.value.start,
          end: Math.min(bounds.value.end, Math.max(value, range.value.start + 60)),
        };
}
function move(edge: 'start' | 'end', event: Event): void {
  update(edge, Number((event.target as HTMLInputElement).value));
}
function commit(): void {
  emit('update:modelValue', { ...range.value });
}
function dateChanged(edge: 'start' | 'end', event: Event): void {
  const value = Date.parse((event.target as HTMLInputElement).value) / 1000;
  if (!Number.isFinite(value)) return;
  update(edge, value);
  commit();
}
function live(): void {
  draft.value = null;
  emit('update:modelValue', null);
}
</script>

<template>
  <section class="range-selector" data-test="chart-range-selector" :aria-label="t('range')">
    <div class="range-heading">
      <span>{{ t('range') }}</span>
      <select v-model.number="contextHours" :aria-label="t('context')" @change="live">
        <option :value="6">{{ t('sixHours') }}</option>
        <option :value="24">{{ t('day') }}</option>
        <option :value="336">{{ t('all') }}</option>
      </select>
      <button type="button" class="btn btn--subtle btn--sm" data-test="range-live" @click="live">
        {{ t('live') }}
      </button>
    </div>
    <template v-if="samples.length">
      <div class="range-track">
        <svg viewBox="0 0 1000 60" preserveAspectRatio="none" aria-hidden="true">
          <path :d="preview" fill="none" stroke="currentColor" vector-effect="non-scaling-stroke" />
        </svg>
        <div class="range-selection" :style="selectionStyle" />
        <div class="range-shade" :style="{ left: '0', width: selectionStyle.left }" />
        <div
          class="range-shade"
          :style="{ left: `calc(${selectionStyle.left} + ${selectionStyle.width})`, right: '0' }"
        />
        <input
          v-for="edge in ['start', 'end'] as const"
          :key="edge"
          type="range"
          :min="bounds.start"
          :max="bounds.end"
          step="60"
          :value="range[edge]"
          :aria-label="t(edge)"
          :aria-valuetext="describeTime(range[edge])"
          :data-test="`range-${edge}`"
          @input="move(edge, $event)"
          @change="commit"
        />
      </div>
      <div class="range-dates">
        <label v-for="edge in ['start', 'end'] as const" :key="edge">
          <span>{{ t(edge) }}</span>
          <input
            type="datetime-local"
            :value="inputTime(range[edge])"
            :min="inputTime(bounds.start)"
            :max="inputTime(bounds.end)"
            :data-test="`range-date-${edge}`"
            @change="dateChanged(edge, $event)"
          />
        </label>
      </div>
      <p>{{ t('hint') }}</p>
    </template>
    <p v-else>{{ t('empty') }}</p>
  </section>
</template>

<style scoped>
.range-selector {
  margin-top: 16px;
  border-top: 1px solid var(--border-soft);
  padding-top: 12px;
  min-width: 0;
}
.range-heading {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  color: var(--text-muted);
  font-size: 12px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.range-heading select {
  margin-inline-start: auto;
  min-width: 0;
  max-width: 100%;
  font-size: 12px;
}
.range-track {
  position: relative;
  overflow: hidden;
  height: 44px;
  margin: 0 10px 14px;
  background: var(--bg-app);
  border: 1px solid var(--border-soft);
}
.range-track svg {
  position: absolute;
  width: 100%;
  height: 100%;
  color: var(--text-muted);
  opacity: 0.5;
}
.range-selection {
  position: absolute;
  top: 0;
  height: 100%;
  border-block: 1px solid var(--text-muted);
  background: color-mix(in srgb, var(--text-secondary) 12%, transparent);
  pointer-events: none;
}
.range-shade {
  position: absolute;
  top: 0;
  bottom: 0;
  background: color-mix(in srgb, var(--bg-app) 55%, transparent);
  pointer-events: none;
}
.range-track input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  margin: 0;
  padding: 0;
  appearance: none;
  background: transparent;
  pointer-events: none;
}
.range-track input::-webkit-slider-thumb {
  appearance: none;
  width: 16px;
  height: 26px;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background:
    linear-gradient(var(--bg-card), var(--bg-card)) calc(50% - 2px) center / 1px 10px no-repeat,
    linear-gradient(var(--bg-card), var(--bg-card)) calc(50% + 2px) center / 1px 10px no-repeat,
    var(--text-secondary);
  pointer-events: auto;
  cursor: ew-resize;
}
.range-track input::-moz-range-thumb {
  width: 14px;
  height: 24px;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background:
    linear-gradient(var(--bg-card), var(--bg-card)) calc(50% - 2px) center / 1px 10px no-repeat,
    linear-gradient(var(--bg-card), var(--bg-card)) calc(50% + 2px) center / 1px 10px no-repeat,
    var(--text-secondary);
  pointer-events: auto;
  cursor: ew-resize;
}
.range-track input:focus-visible {
  outline: none;
  z-index: 2;
}
.range-track input:focus-visible::-webkit-slider-thumb {
  outline: 2px solid var(--text-secondary);
  outline-offset: 3px;
}
.range-track input:focus-visible::-moz-range-thumb {
  outline: 2px solid var(--text-secondary);
  outline-offset: 3px;
}
.range-dates {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}
.range-dates label {
  display: flex;
  flex-direction: column;
  gap: 5px;
  color: var(--text-muted);
  font-size: 12px;
  min-width: 0;
}
.range-dates input {
  width: 100%;
  min-width: 0;
  padding: 7px;
  border: 1px solid var(--border-soft);
  border-radius: var(--input-radius);
  color: var(--text-primary);
  background: var(--bg-card-soft);
  font: inherit;
}
.range-selector p {
  font-size: 11px;
  color: var(--text-muted);
  margin: 9px 0 0;
  line-height: 1.5;
}
@media (max-width: 500px) {
  .range-dates {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
