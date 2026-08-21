import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { apiClient, type SettingsAgentToken } from '@/api';
import TokenTable from './TokenTable.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, deleteAgent: vi.fn(), updateNodeServiceMetadata: vi.fn() },
  };
});

const mockDeleteAgent = vi.mocked(apiClient.deleteAgent);
const mockUpdateMeta = vi.mocked(apiClient.updateNodeServiceMetadata);

const FAKE_DICT = {
  en: {
    'settings.tokens.title': 'Agent Renewal',
    'settings.tokens.empty': 'No enrolled agents yet.',
    'settings.tokens.node': 'Node',
    'settings.tokens.status': 'Status',
    'settings.tokens.agent': 'Agent',
    'settings.tokens.ip': 'Remote IP',
    'settings.tokens.expires_at': 'Expires at',
    'settings.tokens.remaining': 'Remaining',
    'settings.tokens.service_expires_at': 'Service expiry',
    'settings.tokens.service_unlimited': 'Unlimited',
    'settings.tokens.service_unlimited_short': 'No limit',
    'settings.tokens.renewal_price': 'Renewal price',
    'settings.tokens.renewal_price_placeholder': '$5/mo',
    'settings.tokens.actions': 'Actions',
    'settings.tokens.service_meta_save': 'Save',
    'settings.tokens.service_meta_saving': 'Saving…',
    'settings.tokens.service_meta_saved': 'Saved',
    'settings.tokens.service_meta_failed': 'Save failed: {error}',
    'settings.tokens.delete': 'Delete',
    'settings.tokens.delete_title': 'Delete {node}',
    'settings.tokens.delete_warning': 'This removes the agent.',
    'settings.tokens.delete_cancel': 'Cancel',
    'settings.tokens.delete_confirm': 'Delete agent',
    'settings.tokens.delete_deleting': 'Deleting…',
    'settings.tokens.delete_failed': 'Delete failed: {error}',
    'settings.password.current': 'Current password',
    'settings.security.verification_code': '6-digit code',
    'settings.summary.token_health': 'Token Health',
    'settings.token.no_expiry': 'No expiry',
    'settings.token.expired': 'Expired',
    'settings.duration.days_hours': '{days}d {hours}h',
    'settings.duration.minutes': '{minutes}m',
    'common.online': 'Online',
    'common.offline': 'Offline',
    'common.not_available': 'n/a',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

function agent(over: Partial<SettingsAgentToken>): SettingsAgentToken {
  return {
    node_id: 'n',
    node_label: 'N',
    online: true,
    agent_version: '1.0',
    remote_ip: '10.0.0.1',
    tags: [],
    token_expires_at: '2026-12-01T00:00:00Z',
    token_expires_in_secs: 1_000_000,
    service_expires_at: null,
    service_unlimited: false,
    renewal_price: null,
    traffic_limit_bytes: null,
    traffic_used_bytes: null,
    traffic_accounting: 'bidirectional',
    traffic_throttle_kbps: null,
    geoip_country: null,
    geoip_city: null,
    geoip_latitude: null,
    geoip_longitude: null,
    location_override_country: null,
    location_override_city: null,
    location_override_latitude: null,
    location_override_longitude: null,
    ...over,
  };
}

function mountTable(
  agents: SettingsAgentToken[],
  options: { authEnabled?: boolean; twoFactorEnabled?: boolean } = {},
) {
  return mount(TokenTable, {
    props: {
      agents,
      authEnabled: options.authEnabled ?? true,
      twoFactorEnabled: options.twoFactorEnabled ?? false,
    },
    global: { plugins: [getI18n()] },
  });
}

describe('TokenTable', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    mockDeleteAgent.mockResolvedValue({ ok: true, message: 'Agent removed' });
    mockUpdateMeta.mockResolvedValue({ ok: true, message: 'Saved' });
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
    vi.clearAllMocks();
  });

  it('shows the empty state with no agents', () => {
    expect(mountTable([]).find('[data-test="token-table-empty"]').exists()).toBe(true);
  });

  it('renders a row per agent with severity classes', () => {
    const wrapper = mountTable([
      agent({ node_id: 'a', token_expires_in_secs: 30 * 86400 }), // ok
      agent({ node_id: 'b', token_expires_in_secs: 3 * 86400 }), // expiring
      agent({ node_id: 'c', token_expires_in_secs: -1 }), // expired
    ]);
    const rows = wrapper.findAll('[data-test="token-row"]');
    expect(rows).toHaveLength(3);
    expect(wrapper.find('.tokens .ok').exists()).toBe(true);
    expect(wrapper.find('.tokens .expiring').exists()).toBe(true);
    expect(wrapper.find('.tokens .expired').exists()).toBe(true);
    expect(wrapper.text()).toContain('Expired');
  });

  it('saves editable service expiry and renewal price', async () => {
    const wrapper = mountTable([
      agent({
        node_id: 'a',
        service_expires_at: '2026-12-31T00:00:00Z',
        renewal_price: '$4/mo',
      }),
    ]);

    await wrapper.find('[data-test="service-expiry-input"]').setValue('2027-01-15');
    await wrapper.find('[data-test="renewal-price-input"]').setValue('  $5/mo  ');
    await wrapper.find('[data-test="service-meta-save"]').trigger('click');
    await flushPromises();

    expect(mockUpdateMeta).toHaveBeenCalledWith('a', {
      service_expires_at: '2027-01-15T00:00:00Z',
      service_unlimited: false,
      renewal_price: '$5/mo',
      traffic_limit_bytes: null,
      traffic_accounting: 'bidirectional',
      traffic_throttle_kbps: null,
    });
    expect(wrapper.emitted('saved')).toHaveLength(1);
    expect(wrapper.find('[data-test="service-meta-message"]').text()).toBe('Saved');
  });

  it('can save an unlimited service term', async () => {
    const wrapper = mountTable([agent({ node_id: 'a' })]);

    await wrapper.find('[data-test="service-expiry-input"]').setValue('2027-01-15');
    await wrapper.find('[data-test="service-unlimited-input"]').setValue(true);
    await wrapper.find('[data-test="service-meta-save"]').trigger('click');
    await flushPromises();

    expect(mockUpdateMeta).toHaveBeenCalledWith('a', {
      service_expires_at: null,
      service_unlimited: true,
      renewal_price: null,
      traffic_limit_bytes: null,
      traffic_accounting: 'bidirectional',
      traffic_throttle_kbps: null,
    });
  });

  it('deletes an agent after password confirmation', async () => {
    const wrapper = mountTable([agent({ node_id: 'a', node_label: 'Agent A' })]);

    await wrapper.find('[data-test="delete-agent"]').trigger('click');
    expect(wrapper.find('[data-test="delete-agent-modal"]').exists()).toBe(true);
    await wrapper.find('[data-test="reauth-password"]').setValue('secret');
    await wrapper.find('[data-test="delete-agent-form"]').trigger('submit');
    await flushPromises();

    expect(mockDeleteAgent).toHaveBeenCalledWith('a', { current_password: 'secret' });
    expect(wrapper.emitted('deleted')).toHaveLength(1);
    expect(wrapper.find('[data-test="delete-agent-modal"]').exists()).toBe(false);
  });

  it('uses a verification code to delete when 2FA is enabled', async () => {
    const wrapper = mountTable([agent({ node_id: 'a' })], { twoFactorEnabled: true });

    await wrapper.find('[data-test="delete-agent"]').trigger('click');
    await wrapper.find('[data-test="reauth-code"]').setValue('123456');
    await wrapper.find('[data-test="delete-agent-form"]').trigger('submit');
    await flushPromises();

    expect(wrapper.find('[data-test="reauth-password"]').exists()).toBe(false);
    expect(mockDeleteAgent).toHaveBeenCalledWith('a', { code: '123456' });
  });
});
