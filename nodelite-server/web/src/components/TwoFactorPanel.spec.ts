import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { apiClient } from '@/api';
import type { SettingsAuth } from '@/api';
import TwoFactorPanel from './TwoFactorPanel.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      twoFactorStart: vi.fn(),
      twoFactorEnable: vi.fn(),
      twoFactorDisable: vi.fn(),
    },
  };
});

const mockStart = vi.mocked(apiClient.twoFactorStart);
const mockEnable = vi.mocked(apiClient.twoFactorEnable);
const mockDisable = vi.mocked(apiClient.twoFactorDisable);

const FAKE_DICT = {
  en: {
    'settings.security.2fa': '2FA',
    'settings.security.2fa_note': 'note',
    'settings.security.start_2fa': 'Set up',
    'settings.security.starting_2fa': 'Generating…',
    'settings.security.setup_note': 'setup',
    'settings.security.enable_2fa': 'Enable',
    'settings.security.disable_2fa': 'Disable',
    'settings.security.disable_note': 'disable note',
    'settings.security.cancel_setup': 'Cancel',
    'settings.security.enabling_2fa': 'Enabling…',
    'settings.security.disabling_2fa': 'Disabling…',
    'settings.security.enabled_saved': '2FA enabled.',
    'settings.security.disabled_saved': '2FA disabled.',
    'settings.security.action_failed': 'Failed: {error}',
    'settings.password.current': 'Current password',
    'settings.security.verification_code': 'code',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

function auth(over: Partial<SettingsAuth> = {}): SettingsAuth {
  return {
    enabled: true, username: 'admin', two_factor_enabled: false,
    totp_secret_configured: false, session_ttl_secs: 86_400, pending_ttl_secs: 300, ...over,
  };
}

function mountPanel(a: SettingsAuth) {
  return mount(TwoFactorPanel, { props: { auth: a }, global: { plugins: [getI18n()] } });
}

describe('TwoFactorPanel', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    mockStart.mockReset();
    mockEnable.mockReset();
    mockDisable.mockReset();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve(FAKE_DICT) } as unknown as Response),
    );
    const dummy = createApp(Stub);
    await setupI18n(dummy);
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it('idle: shows the start button', () => {
    const wrapper = mountPanel(auth({ two_factor_enabled: false }));
    expect(wrapper.find('[data-test="start-2fa"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="enable-2fa-form"]').exists()).toBe(false);
  });

  it('start → pending-setup shows QR + secret + enable form', async () => {
    mockStart.mockResolvedValueOnce({ secret: 'SECRET123', otpauth_uri: 'otpauth://x', qr_svg: '<svg data-test="svg"></svg>' });
    const wrapper = mountPanel(auth({ two_factor_enabled: false }));
    await wrapper.find('[data-test="start-2fa"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="totp-secret"]').text()).toBe('SECRET123');
    expect(wrapper.find('[data-test="totp-qr"]').html()).toContain('<svg');
    expect(wrapper.find('[data-test="enable-2fa-form"]').exists()).toBe(true);
  });

  it('enable posts password+secret+code and emits changed', async () => {
    mockStart.mockResolvedValueOnce({ secret: 'S1', otpauth_uri: 'x', qr_svg: '<svg/>' });
    mockEnable.mockResolvedValueOnce({ ok: true, message: '' });
    const wrapper = mountPanel(auth({ two_factor_enabled: false }));
    await wrapper.find('[data-test="start-2fa"]').trigger('click');
    await flushPromises();
    await wrapper.find('[data-test="enable-2fa-form"] [data-test="reauth-password"]').setValue('pw');
    await wrapper.find('[data-test="enable-2fa-form"] [data-test="reauth-code"]').setValue('123456');
    await wrapper.find('[data-test="enable-2fa-form"]').trigger('submit');
    await flushPromises();
    expect(mockEnable).toHaveBeenCalledWith({ current_password: 'pw', secret: 'S1', code: '123456' });
    expect(wrapper.emitted('changed')).toHaveLength(1);
  });

  it('enabled: shows the disable form and posts password+code', async () => {
    mockDisable.mockResolvedValueOnce({ ok: true, message: '' });
    const wrapper = mountPanel(auth({ two_factor_enabled: true }));
    expect(wrapper.find('[data-test="disable-2fa-form"]').exists()).toBe(true);
    await wrapper.find('[data-test="disable-2fa-form"] [data-test="reauth-password"]').setValue('pw');
    await wrapper.find('[data-test="disable-2fa-form"] [data-test="reauth-code"]').setValue('654321');
    await wrapper.find('[data-test="disable-2fa-form"]').trigger('submit');
    await flushPromises();
    expect(mockDisable).toHaveBeenCalledWith({ current_password: 'pw', code: '654321' });
    expect(wrapper.emitted('changed')).toHaveLength(1);
  });

  it('surfaces the server error on a failed enable', async () => {
    const { ApiError } = await import('@/api/client');
    mockStart.mockResolvedValueOnce({ secret: 'S', otpauth_uri: 'x', qr_svg: '<svg/>' });
    mockEnable.mockRejectedValueOnce(new ApiError(400, JSON.stringify({ ok: false, message: 'bad code' })));
    const wrapper = mountPanel(auth({ two_factor_enabled: false }));
    await wrapper.find('[data-test="start-2fa"]').trigger('click');
    await flushPromises();
    await wrapper.find('[data-test="enable-2fa-form"] [data-test="reauth-password"]').setValue('pw');
    await wrapper.find('[data-test="enable-2fa-form"] [data-test="reauth-code"]').setValue('000000');
    await wrapper.find('[data-test="enable-2fa-form"]').trigger('submit');
    await flushPromises();
    expect(wrapper.find('[data-test="settings-message"]').text()).toContain('bad code');
    expect(wrapper.emitted('changed')).toBeUndefined();
  });
});
