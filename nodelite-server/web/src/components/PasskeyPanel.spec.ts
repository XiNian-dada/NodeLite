import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApp, defineComponent, h } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { apiClient } from '@/api';
import { __resetI18nForTest, getI18n, setupI18n } from '@/i18n';
import PasskeyPanel from './PasskeyPanel.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      passkeyRegistrationStart: vi.fn(),
      passkeyRegistrationFinish: vi.fn(),
      deletePasskey: vi.fn(),
    },
  };
});

vi.mock('@/auth/passkey', () => ({
  createPasskey: vi.fn(),
  supportsPasskeys: () => true,
}));

const mockStart = vi.mocked(apiClient.passkeyRegistrationStart);
const mockFinish = vi.mocked(apiClient.passkeyRegistrationFinish);
const mockDelete = vi.mocked(apiClient.deletePasskey);

const DICTIONARY = {
  en: {
    'settings.passkeys.title': 'Passkeys',
    'settings.passkeys.note': 'Use a passkey.',
    'settings.passkeys.requires_2fa': 'Enable 2FA first.',
    'settings.passkeys.unsupported_browser': 'Unsupported.',
    'settings.passkeys.empty': 'No passkeys.',
    'settings.passkeys.label': 'Name',
    'settings.passkeys.label_placeholder': 'MacBook',
    'settings.passkeys.add': 'Add passkey',
    'settings.passkeys.adding': 'Adding…',
    'settings.passkeys.added': 'Added.',
    'settings.passkeys.added_on': 'Added {date}',
    'settings.passkeys.remove': 'Remove',
    'settings.passkeys.remove_note': 'Confirm removal.',
    'settings.passkeys.removing': 'Removing…',
    'settings.passkeys.removed': 'Removed.',
    'settings.passkeys.cancel': 'Cancel',
    'settings.passkeys.action_failed': 'Failed: {error}',
    'settings.security.verification_code': 'Code',
    'settings.password.current': 'Password',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

function auth(passkeys: Array<{ id: string; label: string; created_at: string }> = []) {
  return {
    enabled: true,
    username: 'viewer',
    two_factor_enabled: true,
    totp_secret_configured: true,
    passkeys,
    session_ttl_secs: 86_400,
    pending_ttl_secs: 300,
  };
}

describe('PasskeyPanel', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    mockStart.mockReset();
    mockFinish.mockReset();
    mockDelete.mockReset();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(DICTIONARY) } as Response),
    );
    await setupI18n(createApp(Stub));
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
  });

  it('enrols a passkey only after the browser registration result is returned', async () => {
    const { createPasskey } = await import('@/auth/passkey');
    vi.mocked(createPasskey).mockResolvedValue({
      id: 'credential',
      rawId: 'credential',
      type: 'public-key',
      response: {},
    });
    mockStart.mockResolvedValue({ publicKey: { challenge: 'AQ', user: { id: 'Ag' } } });
    mockFinish.mockResolvedValue({
      id: 'passkey-id',
      label: 'MacBook',
      created_at: '2026-01-01T00:00:00Z',
    });
    const wrapper = mount(PasskeyPanel, {
      props: { auth: auth() },
      global: { plugins: [getI18n()] },
    });

    await wrapper.find('[data-test="add-passkey"]').trigger('click');
    await wrapper.find('input[type="text"]').setValue('MacBook');
    await wrapper.find('[data-test="reauth-code"]').setValue('123456');
    await wrapper.find('[data-test="add-passkey-form"]').trigger('submit');
    await flushPromises();

    expect(mockStart).toHaveBeenCalledWith({ label: 'MacBook', code: '123456' });
    expect(mockFinish).toHaveBeenCalledWith(expect.objectContaining({ id: 'credential' }));
    expect(wrapper.emitted('changed')).toHaveLength(1);
  });

  it('requires a fresh code when removing a passkey', async () => {
    mockDelete.mockResolvedValue({ ok: true, message: 'passkey removed' });
    const wrapper = mount(PasskeyPanel, {
      props: {
        auth: auth([{ id: 'passkey-id', label: 'MacBook', created_at: '2026-01-01T00:00:00Z' }]),
      },
      global: { plugins: [getI18n()] },
    });

    await wrapper.find('[data-test="remove-passkey-passkey-id"]').trigger('click');
    await wrapper.find('[data-test="reauth-code"]').setValue('654321');
    await wrapper.find('[data-test="remove-passkey-form"]').trigger('submit');
    await flushPromises();

    expect(mockDelete).toHaveBeenCalledWith('passkey-id', { code: '654321' });
    expect(wrapper.emitted('changed')).toHaveLength(1);
  });
});
