import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';

import LoginNotification from './LoginNotification.vue';
import { apiClient } from '@/api';
import { LANGUAGE_STORAGE_KEY, __resetI18nForTest, getI18n, setupI18n } from '@/i18n';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, lastLogin: vi.fn() },
  };
});

const mockLastLogin = vi.mocked(apiClient.lastLogin);

const FAKE_DICT = {
  en: {
    'audit.last_login.title': 'Last login',
    'audit.last_login.time': 'Time',
    'audit.last_login.location': 'Location',
    'audit.last_login.ip_address': 'IP address',
    'audit.last_login.hour_ago': '1 hour ago',
    'audit.last_login.hours_ago': '{count} hours ago',
    'audit.last_login.day_ago': '1 day ago',
    'audit.last_login.days_ago': '{count} days ago',
    'audit.last_login.recent': 'Recently',
    'audit.last_login.unknown_location': 'Unknown location',
    'audit.last_login.notice': 'If this was not you, secure your account immediately.',
    'common.close': 'Close',
  },
  'zh-CN': {
    'audit.last_login.title': '上次登录',
    'audit.last_login.time': '时间',
    'audit.last_login.location': '位置',
    'audit.last_login.ip_address': 'IP 地址',
    'audit.last_login.hour_ago': '1 小时前',
    'audit.last_login.hours_ago': '{count} 小时前',
    'audit.last_login.day_ago': '1 天前',
    'audit.last_login.days_ago': '{count} 天前',
    'audit.last_login.recent': '刚刚',
    'audit.last_login.unknown_location': '未知位置',
    'audit.last_login.notice': '如果这次登录不是你本人，请立即保护你的账户安全。',
    'common.close': '关闭',
  },
};

const Stub = defineComponent({ render: () => h('div') });

async function mountNotification() {
  const wrapper = mount(LoginNotification, {
    attachTo: document.body,
    global: { plugins: [getI18n()] },
  });
  await flushPromises();
  return wrapper;
}

describe('LoginNotification', () => {
  beforeEach(async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-06T16:00:00Z'));
    window.localStorage.clear();
    window.sessionStorage.clear();
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'zh-CN');
    __resetI18nForTest();
    mockLastLogin.mockResolvedValue({
      timestamp: '2026-07-06T13:00:00Z',
      ip_address: '182.143.152.197',
      user_agent: null,
      country: 'CN',
      city: 'Chengdu',
      latitude: null,
      longitude: null,
    });
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
    vi.useRealTimers();
    document.body.innerHTML = '';
    window.localStorage.clear();
    window.sessionStorage.clear();
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it('renders translated copy for the active locale', async () => {
    const wrapper = await mountNotification();

    expect(document.body.textContent).toContain('上次登录');
    expect(document.body.textContent).toContain('时间');
    expect(document.body.textContent).toContain('3 小时前');
    expect(document.body.textContent).toContain('位置');
    expect(document.body.textContent).toContain('IP 地址');
    expect(document.body.textContent).toContain('如果这次登录不是你本人');

    wrapper.unmount();
  });

  it('auto-dismisses after a few seconds', async () => {
    const wrapper = await mountNotification();

    expect(document.body.querySelector('[data-test="login-notification"]')).not.toBeNull();

    await vi.advanceTimersByTimeAsync(29_000);
    await flushPromises();

    expect(document.body.querySelector('[data-test="login-notification"]')).not.toBeNull();

    await vi.advanceTimersByTimeAsync(1_000);
    await flushPromises();

    expect(document.body.querySelector('[data-test="login-notification"]')).toBeNull();

    wrapper.unmount();
  });

  it('does not persist a dismissed session when auto-hidden', async () => {
    const wrapper = await mountNotification();

    await vi.advanceTimersByTimeAsync(30_000);
    await flushPromises();

    expect(window.sessionStorage.getItem('nodelite_login_notification_dismissed')).toBeNull();

    wrapper.unmount();

    const remounted = await mountNotification();
    expect(document.body.querySelector('[data-test="login-notification"]')).not.toBeNull();
    remounted.unmount();
  });

  it('persists a dismissed session only after manual close', async () => {
    const wrapper = await mountNotification();

    const closeButton = document.body.querySelector('[data-test="login-notification-close"]');
    expect(closeButton).not.toBeNull();
    closeButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await flushPromises();

    expect(window.sessionStorage.getItem('nodelite_login_notification_dismissed')).toBe('true');
    expect(document.body.querySelector('[data-test="login-notification"]')).toBeNull();

    wrapper.unmount();
  });

  it('uses singular english relative labels when count is one', async () => {
    const localeRef = getI18n().global.locale as unknown as { value: string };
    localeRef.value = 'en';
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, 'en');
    mockLastLogin.mockResolvedValueOnce({
      timestamp: '2026-07-06T15:00:00Z',
      ip_address: '182.143.152.197',
      user_agent: null,
      country: 'CN',
      city: 'Chengdu',
      latitude: null,
      longitude: null,
    });
    const hourly = await mountNotification();

    expect(document.body.textContent).toContain('1 hour ago');
    expect(document.body.textContent).not.toContain('1 hours ago');

    hourly.unmount();

    mockLastLogin.mockResolvedValueOnce({
      timestamp: '2026-07-05T16:00:00Z',
      ip_address: '182.143.152.197',
      user_agent: null,
      country: 'CN',
      city: 'Chengdu',
      latitude: null,
      longitude: null,
    });
    const daily = await mountNotification();

    expect(document.body.textContent).toContain('1 day ago');
    expect(document.body.textContent).not.toContain('1 days ago');

    daily.unmount();
  });
});
