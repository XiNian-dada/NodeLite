import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import NodeSummaryCards from './NodeSummaryCards.vue';

const FAKE_DICT = {
  en: { 'node.disk_usage': 'Disk Usage', 'node.load': 'Load' },
  'zh-CN': { 'node.disk_usage': '磁盘使用', 'node.load': '负载' },
};

const Stub = defineComponent({ render: () => h('div') });

function mountCards(node: ReturnType<typeof makeNodeStatus> | null) {
  return mount(NodeSummaryCards, { props: { node }, global: { plugins: [getI18n()] } });
}

describe('NodeSummaryCards', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response),
    );
    const dummy = createApp(Stub);
    await setupI18n(dummy);
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
  });

  it('renders disk percent and load average', () => {
    // fixture: 40 GB used / 100 GB → 40%, load.one 0.3
    const wrapper = mountCards(makeNodeStatus());
    expect(wrapper.find('[data-test="summary-disk-pct"]').text()).toBe('40%');
    expect(wrapper.find('[data-test="summary-load"]').text()).toBe('0.30');
  });

  it('shows dashes when there is no snapshot', () => {
    const wrapper = mountCards(makeNodeStatus({ snapshot: null }));
    expect(wrapper.find('[data-test="summary-disk-pct"]').text()).toBe('—');
    expect(wrapper.find('[data-test="summary-load"]').text()).toBe('—');
  });
});
