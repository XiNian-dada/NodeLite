import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeSettings } from '@/api/__fixtures__/nodes';
import OpsCard from './OpsCard.vue';

const FAKE_DICT = {
  en: {
    'settings.ops.title': 'Operations',
    'settings.ops.config': 'Config',
    'settings.ops.registry': 'Registry',
    'settings.ops.history': 'History DB',
    'settings.ops.snapshot': 'Snapshot',
    'settings.ops.server_upgrade': 'Server upgrade command:',
    'settings.ops.agent_upgrade': 'Agent upgrade command:',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

describe('OpsCard', () => {
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

  it('renders the ops paths and upgrade commands', () => {
    const settings = makeSettings({
      config_path: '/etc/x.toml',
      updates: {
        latest_release_url: 'u',
        server_upgrade_command: 'do-server-upgrade',
        agent_upgrade_command: 'do-agent-upgrade',
      },
    });
    const wrapper = mount(OpsCard, { props: { settings }, global: { plugins: [getI18n()] } });
    const text = wrapper.text();
    expect(text).toContain('/etc/x.toml');
    expect(text).toContain('do-server-upgrade');
    expect(text).toContain('do-agent-upgrade');
  });
});
