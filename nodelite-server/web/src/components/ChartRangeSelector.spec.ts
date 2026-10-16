import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import type { HistoryPoint } from '@/api';
import ChartRangeSelector from './ChartRangeSelector.vue';

const end = Date.parse('2026-10-16T12:00:00Z') / 1000;
const history = Array.from(
  { length: 25 },
  (_, index) =>
    ({
      recorded_at: new Date((end - (24 - index) * 3600) * 1000).toISOString(),
      rx_bytes_per_sec: index * 100,
      tx_bytes_per_sec: 50,
    }) as HistoryPoint,
);

function mountSelector(points = history) {
  return mount(ChartRangeSelector, {
    props: { history: points, modelValue: null },
    global: { plugins: [createI18n({ legacy: false, locale: 'en' })] },
  });
}

describe('chart time-range navigator', () => {
  it('defaults to the latest hour with enough navigator space for separate handles', () => {
    const wrapper = mountSelector();
    const start = wrapper.get<HTMLInputElement>('[data-test="range-start"]');
    const finish = wrapper.get<HTMLInputElement>('[data-test="range-end"]');
    expect(Number(start.element.value)).toBe(end - 3600);
    expect(Number(finish.element.value)).toBe(end);
    expect(Number(start.attributes('min'))).toBe(end - 6 * 3600);
  });

  it('does not fetch on every drag movement and commits an exact range on release', async () => {
    const wrapper = mountSelector();
    const start = wrapper.get<HTMLInputElement>('[data-test="range-start"]');
    start.element.value = String(end - 7200);
    await start.trigger('input');
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    await start.trigger('change');
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([{ start: end - 7200, end }]);
  });

  it('prevents reversed or empty ranges and can return to following the latest hour', async () => {
    const wrapper = mountSelector();
    await wrapper.get('[data-test="range-start"]').setValue(String(end));
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([{ start: end - 60, end }]);
    await wrapper.get('[data-test="range-live"]').trigger('click');
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([null]);
  });

  it('keeps an explicit historical selection pinned as new samples arrive', async () => {
    const wrapper = mountSelector();
    await wrapper.setProps({ modelValue: { start: end - 7200, end: end - 3600 } });
    await wrapper.setProps({
      history: [
        ...history,
        { ...history[0]!, recorded_at: new Date((end + 300) * 1000).toISOString() },
      ],
    });
    expect(Number(wrapper.get<HTMLInputElement>('[data-test="range-end"]').element.value)).toBe(
      end - 3600,
    );
  });

  it('handles empty and invalid timestamps without invalid slider coordinates', () => {
    const wrapper = mountSelector([{ ...history[0]!, recorded_at: 'invalid' }]);
    expect(wrapper.find('input[type="range"]').exists()).toBe(false);
    expect(wrapper.text()).toContain('Waiting for history');
  });

  it('does not silently shift a pinned historical range when the live context advances by a day', async () => {
    const wrapper = mountSelector();
    await wrapper.setProps({ modelValue: { start: end - 7200, end: end - 3600 } });
    await wrapper.setProps({
      history: [
        ...history,
        { ...history[0]!, recorded_at: new Date((end + 86400) * 1000).toISOString() },
      ],
    });
    expect(Number(wrapper.get<HTMLInputElement>('[data-test="range-start"]').element.value)).toBe(
      end - 7200,
    );
    expect(Number(wrapper.get<HTMLInputElement>('[data-test="range-end"]').element.value)).toBe(
      end - 3600,
    );
  });
});
