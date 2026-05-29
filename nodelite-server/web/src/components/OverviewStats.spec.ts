import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { useOverviewStore } from '@/stores/overview';
import { makeOverview } from '@/api/__fixtures__/nodes';
import OverviewStats from './OverviewStats.vue';

const FAKE_DICT = {
  en: {
    'index.stat.total': 'Total Servers',
    'index.stat.online': 'Online',
    'index.stat.offline': 'Offline',
    'index.stat.latency': 'Avg Latency',
  },
  'zh-CN': {
    'index.stat.total': '服务器总数',
    'index.stat.online': '在线',
    'index.stat.offline': '离线',
    'index.stat.latency': '平均延迟',
  },
};

const Stub = defineComponent({ render: () => h('div') });

async function mountWith(data: ReturnType<typeof makeOverview> | null) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useOverviewStore();
  store.data = data;
  const wrapper = mount(OverviewStats, { global: { plugins: [pinia, getI18n()] } });
  await wrapper.vm.$nextTick();
  return wrapper;
}

describe('OverviewStats', () => {
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

  it('shows placeholders when the store has no data', async () => {
    const wrapper = await mountWith(null);
    expect(wrapper.find('[data-test="stat-total"]').text()).toBe('--');
    expect(wrapper.find('[data-test="stat-latency"]').text()).toContain('--');
  });

  it('renders the overview numbers', async () => {
    const wrapper = await mountWith(
      makeOverview({
        total_nodes: 12,
        online_nodes: 10,
        offline_nodes: 2,
        average_latency_ms: 8.6,
      }),
    );
    expect(wrapper.find('[data-test="stat-total"]').text()).toBe('12');
    expect(wrapper.find('[data-test="stat-online"]').text()).toBe('10');
    expect(wrapper.find('[data-test="stat-offline"]').text()).toBe('2');
    // 8.6 rounds to 9 ms
    expect(wrapper.find('[data-test="stat-latency"]').text()).toContain('9');
  });

  it('shows -- for null average latency', async () => {
    const wrapper = await mountWith(makeOverview({ average_latency_ms: null }));
    expect(wrapper.find('[data-test="stat-latency"]').text()).toContain('--');
  });
});
