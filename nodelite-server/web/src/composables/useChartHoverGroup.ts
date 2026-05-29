import { inject, provide, ref, type InjectionKey, type Ref } from 'vue';

/**
 * Linked-hover group (reactive replacement for legacy createHoverGroup,
 * node.html:2222). Charts in the same group share an `activeTs`: hovering one
 * sets it, and every member renders its crosshair at that timestamp. A parent
 * (e.g. MonitorCharts/OverviewCharts) calls provideChartHoverGroup once;
 * MetricChart injects it (optional — a standalone chart has no group).
 */
export interface ChartHoverGroup {
  activeTs: Ref<number | null>;
  set(ts: number | null): void;
}

const CHART_HOVER_GROUP: InjectionKey<ChartHoverGroup> = Symbol('chartHoverGroup');

export function provideChartHoverGroup(): ChartHoverGroup {
  const activeTs = ref<number | null>(null);
  const group: ChartHoverGroup = {
    activeTs,
    set: (ts) => {
      activeTs.value = ts;
    },
  };
  provide(CHART_HOVER_GROUP, group);
  return group;
}

export function useChartHoverGroup(): ChartHoverGroup | null {
  return inject(CHART_HOVER_GROUP, null);
}
