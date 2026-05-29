import { computed, onBeforeUnmount, onMounted, ref, watch, type ComputedRef, type Ref } from 'vue';
import { computeHover, nearestByKey, type HoverModel } from '@/lib/chart/hover';
import type { ChartModel } from '@/lib/chart/svgModel';
import { useChartHoverGroup } from './useChartHoverGroup';

export interface UseChartOptions {
  fallbackWidth?: number;
  fallbackHeight: number;
  formatTime: (ts: number) => string;
}

export interface UseChartReturn {
  model: ComputedRef<ChartModel>;
  hover: Ref<HoverModel | null>;
  onPointerMove: (event: PointerEvent) => void;
  onPointerLeave: () => void;
}

/**
 * Wires a chart's reactive size + hover to the pure builders. Owns width/
 * height (ResizeObserver, so the legacy no-observer gap is closed) and feeds
 * them to `buildModel`. Hover: a standalone chart resolves by mouse-x; a
 * grouped chart publishes the anchor ts to the shared group and every member
 * (incl. the source) renders its crosshair by ts (linked hover).
 */
export function useChart(
  containerRef: Ref<HTMLElement | null>,
  buildModel: (size: { width: number; height: number }) => ChartModel,
  opts: UseChartOptions,
): UseChartReturn {
  const width = ref(opts.fallbackWidth ?? 600);
  const height = ref(opts.fallbackHeight);
  const model = computed(() => buildModel({ width: width.value, height: height.value }));
  const hover = ref<HoverModel | null>(null);
  const group = useChartHoverGroup();

  let observer: ResizeObserver | null = null;

  function measure(): void {
    const el = containerRef.value;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.width > 0) width.value = Math.round(rect.width);
    if (rect.height > 0) height.value = Math.round(rect.height);
  }

  onMounted(() => {
    measure();
    if (typeof ResizeObserver !== 'undefined' && containerRef.value) {
      observer = new ResizeObserver(() => measure());
      observer.observe(containerRef.value);
    }
  });

  onBeforeUnmount(() => {
    observer?.disconnect();
    observer = null;
    if (group) group.set(null);
  });

  function renderByTs(ts: number | null): void {
    if (ts == null) {
      hover.value = null;
      return;
    }
    const el = containerRef.value;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    hover.value = computeHover(model.value, ts, 'ts', rect, opts.formatTime);
  }

  if (group) {
    watch(group.activeTs, (ts) => renderByTs(ts));
  }

  function onPointerMove(event: PointerEvent): void {
    const el = containerRef.value;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return;
    const viewX = Math.min(
      model.value.width,
      Math.max(0, ((event.clientX - rect.left) / rect.width) * model.value.width),
    );
    if (group) {
      // Find the anchor ts under the cursor and broadcast; the watch above
      // (on every member, incl. this one) renders the crosshair by ts.
      let anchorTs: number | null = null;
      for (const series of model.value.series) {
        const p = nearestByKey(series.points, viewX, 'x');
        if (p && p.ts != null) {
          anchorTs = p.ts;
          break;
        }
      }
      if (anchorTs == null) {
        hover.value = computeHover(model.value, viewX, 'x', rect, opts.formatTime);
      } else {
        group.set(anchorTs);
      }
    } else {
      hover.value = computeHover(model.value, viewX, 'x', rect, opts.formatTime);
    }
  }

  function onPointerLeave(): void {
    if (group) group.set(null);
    else hover.value = null;
  }

  return { model, hover, onPointerMove, onPointerLeave };
}
