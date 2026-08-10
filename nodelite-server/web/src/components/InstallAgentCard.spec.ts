import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { apiClient } from '@/api';
import { makeSettings } from '@/api/__fixtures__/nodes';
import { __resetI18nForTest, getI18n, setupI18n } from '@/i18n';
import InstallAgentCard from './InstallAgentCard.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return { ...actual, apiClient: { ...actual.apiClient, generateAgentInstall: vi.fn() } };
});

const mockGenerate = vi.mocked(apiClient.generateAgentInstall);
const FAKE_DICT = {
  en: {
    'settings.install.kicker': 'Agents',
    'settings.install.title': 'Install Agent',
    'settings.install.note': 'Create a one-time command.',
    'settings.install.open': 'Install Agent',
    'settings.install.unavailable': 'Authentication required.',
    'settings.install.node_id': 'Node ID',
    'settings.install.node_label': 'Node label',
    'settings.install.tags': 'Tags',
    'settings.install.tags_placeholder': 'edge, prod',
    'settings.install.node_id_required': 'Node ID is required.',
    'settings.install.generating': 'Generating…',
    'settings.install.generate': 'Generate command',
    'settings.install.generated': 'Generated',
    'settings.install.generate_failed': 'Generation failed: {error}',
    'settings.install.command_title': 'Install {node}',
    'settings.install.expires_at': 'Expires {time}',
    'settings.install.copy': 'Copy command',
    'settings.install.copied': 'Copied',
    'settings.install.copy_failed': 'Copy failed',
    'settings.install.rotate_notice': 'Existing nodes rotate their token.',
    'settings.password.current': 'Current password',
    'settings.security.verification_code': '6-digit code',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

async function mountCard({ twoFactorEnabled = false, authEnabled = true } = {}) {
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
  const settings = makeSettings();
  settings.auth.two_factor_enabled = twoFactorEnabled;
  settings.auth.enabled = authEnabled;
  return mount(InstallAgentCard, {
    props: { settings },
    global: { plugins: [getI18n()] },
  });
}

describe('InstallAgentCard', () => {
  beforeEach(() => {
    __resetI18nForTest();
    mockGenerate.mockReset();
    mockGenerate.mockResolvedValue({
      ok: true,
      message: 'command ready',
      node_id: 'sg-01',
      node_label: 'Singapore 01',
      created: true,
      install_token_expires_at: '2026-12-01T00:00:00Z',
      install_command: 'curl -fsSL https://monitor.example/install-agent.sh | sh',
    });
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it('collects node details and renders the generated command', async () => {
    const wrapper = await mountCard();
    await wrapper.find('[data-test="open-install-agent"]').trigger('click');
    await wrapper.find('[data-test="install-agent-node-id"]').setValue(' sg-01 ');
    await wrapper.find('[data-test="install-agent-node-label"]').setValue(' Singapore 01 ');
    await wrapper.find('[data-test="install-agent-tags"]').setValue(' edge, prod, ');
    await wrapper.find('[data-test="reauth-password"]').setValue('secret');
    await wrapper.find('[data-test="install-agent-form"]').trigger('submit');
    await flushPromises();

    expect(mockGenerate).toHaveBeenCalledWith({
      node_id: 'sg-01',
      node_label: 'Singapore 01',
      tags: ['edge', 'prod'],
      current_password: 'secret',
    });
    expect(wrapper.find('[data-test="install-agent-command"]').text()).toContain('curl -fsSL');
    expect(wrapper.emitted('created')).toHaveLength(1);
  });

  it('uses a verification code instead of the password when 2FA is enabled', async () => {
    const wrapper = await mountCard({ twoFactorEnabled: true });
    await wrapper.find('[data-test="open-install-agent"]').trigger('click');
    await wrapper.find('[data-test="install-agent-node-id"]').setValue('sg-01');
    await wrapper.find('[data-test="reauth-code"]').setValue('123456');
    await wrapper.find('[data-test="install-agent-form"]').trigger('submit');
    await flushPromises();

    expect(wrapper.find('[data-test="reauth-password"]').exists()).toBe(false);
    expect(mockGenerate).toHaveBeenCalledWith({
      node_id: 'sg-01',
      tags: [],
      code: '123456',
    });
  });

  it('requires dashboard authentication before exposing an installation command', async () => {
    const wrapper = await mountCard({ authEnabled: false });
    expect(wrapper.find('[data-test="install-agent-unavailable"]').exists()).toBe(true);
    expect(
      (wrapper.find('[data-test="open-install-agent"]').element as HTMLButtonElement).disabled,
    ).toBe(true);
  });
});
