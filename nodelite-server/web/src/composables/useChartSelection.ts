import { computed, ref, type ComputedRef, type Ref } from 'vue';

/** Monitor-tab window presets, from node.html:1421-1427. */
export const PRESET_WINDOWS = [
  { key: 'last_3h', hours: 3 },
  { key: 'last_24h', hours: 24 },
  { key: 'last_3d', hours: 72 },
  { key: 'last_7d', hours: 168 },
  { key: 'last_14d', hours: 336 },
] as const;

export type PresetKey = (typeof PRESET_WINDOWS)[number]['key'];

const DEFAULT_PRESET: PresetKey = 'last_24h';

export interface ChartSelection {
  presets: typeof PRESET_WINDOWS;
  activeKey: Ref<PresetKey>;
  windowHours: ComputedRef<number>;
  selectPreset: (key: PresetKey) => void;
}

/**
 * Monitor window selection. Holds the active preset; windowHours drives the
 * high-res history fetch. (Freehand brush-drag range selection is a deferred
 * enhancement; presets cover the primary window UX.)
 */
export function useChartSelection(): ChartSelection {
  const activeKey = ref<PresetKey>(DEFAULT_PRESET);
  const windowHours = computed(
    () => PRESET_WINDOWS.find((p) => p.key === activeKey.value)?.hours ?? 24,
  );
  function selectPreset(key: PresetKey): void {
    activeKey.value = key;
  }
  return { presets: PRESET_WINDOWS, activeKey, windowHours, selectPreset };
}
