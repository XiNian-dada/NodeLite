import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import type { SettingsAgentToken } from '@/api';
import TokenTable from './TokenTable.vue';

const FAKE_DICT = {
  en: {
    'settings.tokens.title': 'Agent Token Expiry',
    'settings.tokens.empty': 'No enrolled agents yet.',
    'settings.tokens.node': 'Node',
    'settings.tokens.status': 'Status',
    'settings.tokens.agent': 'Agent',
    'settings.tokens.ip': 'Remote IP',
    'settings.tokens.expires_at': 'Expires at',
    'settings.tokens.remaining': 'Remaining',
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
    node_id: 'n', node_label: 'N', online: true, agent_version: '1.0',
    remote_ip: '10.0.0.1', tags: [], token_expires_at: '2026-12-01T00:00:00Z',
    token_expires_in_secs: 1_000_000, ...over,
  };
}

function mountTable(agents: SettingsAgentToken[]) {
  return mount(TokenTable, { props: { agents }, global: { plugins: [getI18n()] } });
}

describe('TokenTable', () => {
  beforeEach(async () => {
    __resetI18nForTest();
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
});
