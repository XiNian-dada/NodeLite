import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { apiClient } from '@/api';
import { makeSettings } from '@/api/__fixtures__/nodes';
import { authenticatePasskey } from '@/auth/passkey';
import { finishStepUp } from '@/auth/stepUp';
import { __resetI18nForTest, getI18n, setupI18n } from '@/i18n';
import StepUpDialog from './StepUpDialog.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      settings: vi.fn(),
      confirmSettings: vi.fn(),
      confirmSettingsPasskeyStart: vi.fn(),
      confirmSettingsPasskeyFinish: vi.fn(),
    },
  };
});
vi.mock('@/auth/passkey', () => ({ authenticatePasskey: vi.fn(), supportsPasskeys: () => true }));
vi.mock('@/auth/stepUp', () => ({ finishStepUp: vi.fn() }));

const dictionary = {
  en: {
    'settings.confirm.title': 'Confirm',
    'settings.confirm.note': 'Confirm for five minutes',
    'settings.confirm.confirm': 'Confirm',
    'settings.confirm.or': 'or',
    'settings.confirm.passkey': 'Use passkey',
    'settings.security.verification_code': '6-digit code',
    'settings.password.current': 'Current password',
    'settings.passkeys.cancel': 'Cancel',
  },
  'zh-CN': {},
};

async function mountDialog(
  twoFactor: boolean,
  passkeys = twoFactor ? [{ id: 'one', label: 'Mac', created_at: '2026-01-01' }] : [],
) {
  __resetI18nForTest();
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(dictionary) }),
  );
  const app = createApp(defineComponent({ render: () => h('div') }));
  await setupI18n(app);
  const settings = makeSettings();
  settings.auth.two_factor_enabled = twoFactor;
  settings.auth.passkeys = passkeys;
  vi.mocked(apiClient.settings).mockResolvedValue(settings);
  const wrapper = mount(StepUpDialog, {
    global: {
      plugins: [getI18n()],
      stubs: { NativeDialog: { template: '<div><slot /></div>' } },
    },
  });
  await flushPromises();
  return wrapper;
}

describe('StepUpDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(apiClient.confirmSettings).mockResolvedValue({ ok: true, message: '' });
    vi.mocked(apiClient.confirmSettingsPasskeyStart).mockResolvedValue({
      publicKey: { challenge: 'AQ' },
    });
    vi.mocked(authenticatePasskey).mockResolvedValue({
      id: 'credential',
      rawId: 'AQ',
      type: 'public-key',
      response: {},
    });
    vi.mocked(apiClient.confirmSettingsPasskeyFinish).mockResolvedValue({ ok: true, message: '' });
  });

  it('offers TOTP and passkey as alternative choices', async () => {
    const wrapper = await mountDialog(true);
    expect(wrapper.find('[data-test="step-up-passkey"]').exists()).toBe(true);
    await wrapper.find('[data-test="step-up-code"]').setValue('123456');
    await wrapper.find('form').trigger('submit');
    await flushPromises();
    expect(apiClient.confirmSettings).toHaveBeenCalledWith({ code: '123456' });
    expect(finishStepUp).toHaveBeenCalledWith(true);
    wrapper.unmount();
  });

  it('uses a passkey without submitting a TOTP code', async () => {
    const wrapper = await mountDialog(true);
    await wrapper.find('[data-test="step-up-passkey"]').trigger('click');
    await flushPromises();
    expect(apiClient.confirmSettingsPasskeyFinish).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'credential' }),
    );
    expect(apiClient.confirmSettings).not.toHaveBeenCalled();
    expect(finishStepUp).toHaveBeenCalledWith(true);
    wrapper.unmount();
  });

  it('asks for the password when 2FA is disabled', async () => {
    const wrapper = await mountDialog(false, []);
    expect(wrapper.find('[data-test="step-up-passkey"]').exists()).toBe(false);
    await wrapper.find('[data-test="step-up-password"]').setValue('secret');
    await wrapper.find('form').trigger('submit');
    await flushPromises();
    expect(apiClient.confirmSettings).toHaveBeenCalledWith({ current_password: 'secret' });
    wrapper.unmount();
  });
});
