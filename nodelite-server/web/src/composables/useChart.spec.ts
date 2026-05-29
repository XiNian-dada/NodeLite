import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { defineComponent, h, ref } from 'vue';
import type { ChartPoint } from '@/lib/chart/chartData';
import { buildAreaChart } from '@/lib/chart/svgModel';
import { useChart } from './useChart';

function pts(values: number[]): ChartPoint[] {
  return values.map((value, i) => ({ ts: i * 60_000, value }));
}

const Host = defineComponent({
  setup() {
    const containerRef = ref<HTMLElement | null>(null);
    const chart = useChart(
      containerRef,
      ({ width, height }) =>
        buildAreaChart(pts([10, 50, 90]), {
          width,
          height,
          valueKind: 'percent',
          color: 'var(--chart-cpu)',
          label: 'CPU',
        }),
      { fallbackHeight: 200, formatTime: (ts) => `@${ts}` },
    );
    return { containerRef, ...chart };
  },
  render() {
    return h('div', { ref: 'containerRef' });
  },
});

describe('useChart (standalone hover wiring)', () => {
  beforeEach(() => {
    // jsdom returns a zero rect; pretend the chart is 600x200 on screen.
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      width: 600,
      height: 200,
      left: 0,
      top: 0,
      right: 600,
      bottom: 200,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('builds a model sized from the measured container', () => {
    const wrapper = mount(Host);
    expect(wrapper.vm.model.width).toBe(600);
    expect(wrapper.vm.model.height).toBe(200);
    expect(wrapper.vm.model.empty).toBe(false);
  });

  it('sets hover on pointer move and clears it on leave', async () => {
    const wrapper = mount(Host);
    expect(wrapper.vm.hover).toBeNull();

    wrapper.vm.onPointerMove({ clientX: 300 } as PointerEvent);
    await wrapper.vm.$nextTick();
    expect(wrapper.vm.hover).not.toBeNull();
    expect(wrapper.vm.hover!.tooltip.rows[0]!.label).toBe('CPU');

    wrapper.vm.onPointerLeave();
    expect(wrapper.vm.hover).toBeNull();
  });
});
