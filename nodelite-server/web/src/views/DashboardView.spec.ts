import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter, type Router } from 'vue-router';
import { createApp, defineComponent, h } from 'vue';

import DashboardView from './DashboardView.vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { __resetWorldGeoJsonForTest } from '@/composables/useWorldGeoJson';

const FAKE_DICT = {
  en: {
    'index.heading': 'Overview',
    'index.subtitle': 'Global server monitoring · {count} online',
    'common.waiting_for_data': 'Waiting for data…',
    'common.theme_toggle': 'Toggle theme',
    'common.language': 'Language',
    'index.nav.overview': 'Overview',
    'index.nav.settings': 'Settings',
    'index.nav.alerts': 'Alerts',
    'index.nav.account': 'Account',
    'index.stat.time': 'Current Time',
    'index.stat.total': 'Total Servers',
    'index.stat.online': 'Online',
    'index.stat.online_ratio': 'Online Now',
    'index.stat.regions': 'Active Regions',
    'index.stat.offline': 'Offline',
    'index.stat.latency': 'Avg Latency',
    'index.stat.avg_load': 'Avg Load',
    'index.map.title': 'Global Distribution',
    'index.map.legend_online': 'Online',
    'index.map.legend_latency': 'High latency',
    'index.map.legend_offline': 'Offline',
    'index.matrix.title': 'Load Overview',
    'index.matrix.subtitle': 'Current average load per node',
    'index.matrix.more': 'More',
    'index.matrix.col_node': 'Node',
    'index.matrix.col_current': 'Now',
    'index.matrix.col_current_load': 'Current Load',
    'index.matrix.col_status': 'Status',
    'index.matrix.empty': 'No agents reporting yet.',
    'index.node.load': 'Load',
    'index.node.latency': 'Latency',
    'index.node.cpu': 'CPU',
    'index.node.memory': 'Memory',
    'index.node.memory_used': 'Memory',
    'index.node.service_expiry': 'Service expiry',
    'index.node.renewal_price': 'Renewal price',
    'index.node.service_unlimited': 'Unlimited',
    'index.node.self_owned': 'Self-owned',
    'common.online': 'Online',
    'common.offline': 'Offline',
    'common.latency_warn': 'High latency',
  },
  'zh-CN': {
    'index.heading': '概览',
    'index.subtitle': '全球服务器监控 · {count} 在线',
    'common.waiting_for_data': '等待数据…',
    'common.theme_toggle': '切换主题',
    'common.language': '语言',
    'index.nav.overview': '概览',
    'index.nav.settings': '设置',
    'index.nav.alerts': '告警',
    'index.nav.account': '账户',
    'index.stat.time': '当前时间',
    'index.stat.total': '服务器总数',
    'index.stat.online': '在线',
    'index.stat.online_ratio': '当前在线',
    'index.stat.regions': '点亮地区',
    'index.stat.offline': '离线',
    'index.stat.latency': '平均延迟',
    'index.stat.avg_load': '平均负载',
    'index.map.title': '全球分布',
    'index.map.legend_online': '在线',
    'index.map.legend_latency': '延迟偏高',
    'index.map.legend_offline': '离线',
    'index.matrix.title': '负载概览',
    'index.matrix.subtitle': '节点近期平均负载',
    'index.matrix.more': '更多',
    'index.matrix.col_node': '节点',
    'index.matrix.col_current': '当前',
    'index.matrix.col_current_load': '当前负载',
    'index.matrix.col_status': '状态',
    'index.matrix.empty': '暂无节点接入。',
    'index.node.load': '负载',
    'index.node.latency': '延迟',
    'index.node.cpu': 'CPU',
    'index.node.memory': '内存',
    'index.node.memory_used': '内存',
    'index.node.service_expiry': '服务到期',
    'index.node.renewal_price': '续费价格',
    'index.node.service_unlimited': '无限制',
    'index.node.self_owned': '自持有',
    'common.online': '在线',
    'common.offline': '离线',
    'common.latency_warn': '高延迟',
  },
};

const Stub = defineComponent({ render: () => h('div') });

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', name: 'dashboard', component: Stub },
      { path: '/nodes/:id', name: 'node-detail', component: Stub },
      { path: '/settings', name: 'settings', component: Stub },
      { path: '/alerts', name: 'alerts', component: Stub },
      { path: '/account', name: 'account', component: Stub },
    ],
  });
}

async function mountDashboard() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const router = makeRouter();
  await router.push('/');
  await router.isReady();
  const wrapper = mount(DashboardView, {
    global: { plugins: [pinia, router, getI18n()] },
  });
  await flushPromises();
  return wrapper;
}

describe('DashboardView', () => {
  beforeEach(async () => {
    window.localStorage.clear();
    __resetI18nForTest();
    __resetWorldGeoJsonForTest();
    // jsdom has no canvas 2D context; NodeMap's paint no-ops with null.
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
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
    window.localStorage.clear();
    __resetI18nForTest();
    __resetWorldGeoJsonForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    delete document.documentElement.dataset.theme;
  });

  it('renders inside AppLayout with map, stats, and node list', async () => {
    const wrapper = await mountDashboard();
    // AppLayout chrome (theme/lang coverage lives in AppLayout.spec).
    expect(wrapper.find('[data-test="app-shell"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="sidebar-nav"]').exists()).toBe(true);
    // Dashboard body.
    expect(wrapper.find('[data-test="dashboard-view"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-map"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="overview-stats"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-health-matrix"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-list"]').exists()).toBe(true);
  });
});
