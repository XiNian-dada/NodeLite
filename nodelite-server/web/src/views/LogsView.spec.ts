import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter, type Router } from 'vue-router';
import { createApp, defineComponent, h } from 'vue';

import LogsView from './LogsView.vue';
import { apiClient } from '@/api';
import { LANGUAGE_STORAGE_KEY, __resetI18nForTest, getI18n, setupI18n } from '@/i18n';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, auditLog: vi.fn() },
  };
});

const mockAuditLog = vi.mocked(apiClient.auditLog);

const FAKE_DICT = {
  en: {
    'audit.heading': 'Audit log',
    'audit.subtitle': 'Security and authentication events',
    'audit.loading': 'Loading audit log…',
    'audit.empty': 'No audit log entries found.',
    'audit.load_failed': 'Failed to load audit log: {error}',
    'audit.columns.time': 'Time',
    'audit.columns.event': 'Event',
    'audit.columns.user': 'User',
    'audit.columns.ip_address': 'IP address',
    'audit.columns.location': 'Location',
    'audit.columns.status': 'Status',
    'audit.events.login_success': 'Login success',
    'audit.events.login_failure': 'Login failure',
    'audit.events.totp_verify_success': '2FA verified',
    'audit.events.totp_verify_failure': '2FA failed',
    'audit.events.node_connected': 'Node connected',
    'audit.events.token_invalid': 'Invalid token',
    'audit.events.rate_limit_exceeded': 'Rate limited',
    'audit.status.success': 'Success',
    'audit.status.failure': 'Failure',
    'common.language': 'Language',
    'common.theme_toggle': 'Toggle theme',
    'index.nav.overview': 'Overview',
    'index.nav.settings': 'Settings',
    'index.nav.alerts': 'Alerts',
    'index.nav.logs': 'Logs',
    'index.nav.account': 'Account',
  },
  'zh-CN': {
    'audit.heading': '审计日志',
    'audit.subtitle': '安全与认证事件',
    'audit.loading': '正在加载审计日志…',
    'audit.empty': '暂无审计日志。',
    'audit.load_failed': '加载审计日志失败：{error}',
    'audit.columns.time': '时间',
    'audit.columns.event': '事件',
    'audit.columns.user': '用户',
    'audit.columns.ip_address': 'IP 地址',
    'audit.columns.location': '位置',
    'audit.columns.status': '状态',
    'audit.events.login_success': '登录成功',
    'audit.events.login_failure': '登录失败',
    'audit.events.totp_verify_success': '二步验证通过',
    'audit.events.totp_verify_failure': '二步验证失败',
    'audit.events.node_connected': '节点已连接',
    'audit.events.token_invalid': 'Token 无效',
    'audit.events.rate_limit_exceeded': '触发限流',
    'audit.status.success': '成功',
    'audit.status.failure': '失败',
    'common.language': '语言',
    'common.theme_toggle': '切换主题',
    'index.nav.overview': '概览',
    'index.nav.settings': '设置',
    'index.nav.alerts': '告警',
    'index.nav.logs': '日志',
    'index.nav.account': '账户',
  },
};

const Stub = defineComponent({ render: () => h('div') });

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: Stub },
      { path: '/nodes/:id', component: Stub },
      { path: '/settings', component: Stub },
      { path: '/alerts', component: Stub },
      { path: '/logs', name: 'logs', component: LogsView },
      { path: '/account', component: Stub },
    ],
  });
}

async function mountView() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const router = makeRouter();
  await router.push('/logs');
  await router.isReady();
  const wrapper = mount(LogsView, {
    global: { plugins: [pinia, router, getI18n()] },
  });
  await flushPromises();
  return wrapper;
}

describe('LogsView', () => {
  beforeEach(async () => {
    window.localStorage.clear();
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'zh-CN');
    __resetI18nForTest();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response),
    );
    await setupI18n(createApp(Stub));
  });

  afterEach(() => {
    window.localStorage.clear();
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it('renders translated audit copy for the active locale', async () => {
    mockAuditLog.mockResolvedValue([
      {
        id: 1,
        timestamp: '2026-07-06T15:13:05Z',
        event_type: 'login_success',
        user: 'xinian',
        node_id: null,
        ip_address: '211.83.127.156',
        user_agent: null,
        success: true,
        details: { city: 'Chengdu', country: 'CN' },
      },
    ]);

    const wrapper = await mountView();

    expect(wrapper.text()).toContain('审计日志');
    expect(wrapper.text()).toContain('安全与认证事件');
    expect(wrapper.text()).toContain('时间');
    expect(wrapper.text()).toContain('事件');
    expect(wrapper.text()).toContain('IP 地址');
    expect(wrapper.text()).toContain('登录成功');
    expect(wrapper.text()).toContain('成功');
  });

  it('renders translated load errors', async () => {
    mockAuditLog.mockRejectedValueOnce(new Error('boom'));

    const wrapper = await mountView();

    expect(wrapper.find('[data-test="logs-error"]').text()).toBe('加载审计日志失败：boom');
  });
});
