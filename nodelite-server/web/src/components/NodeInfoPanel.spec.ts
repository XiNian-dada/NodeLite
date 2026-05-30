import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import NodeInfoPanel from './NodeInfoPanel.vue';

const FAKE_DICT = {
  en: {
    'node.info.title': 'Server Info',
    'node.info.os': 'OS',
    'node.info.kernel': 'Kernel',
    'node.info.cpu': 'CPU',
    'node.info.memory': 'Memory',
    'node.info.disk': 'Disk',
    'node.info.virtualization': 'Agent',
    'node.info.uptime': 'Uptime',
    'node.info.cores': '{count} Core(s)',
    'node.uptime.days_hours': '{days}d {hours}h {minutes}m',
    'node.uptime.hours_minutes': '{hours}h {minutes}m',
    'node.uptime.minutes': '{minutes}m',
    'common.unknown': 'Unknown',
    'common.unknown_os': 'unknown os',
    'common.not_available': 'n/a',
  },
  'zh-CN': { 'node.info.title': '服务器信息' },
};

const Stub = defineComponent({ render: () => h('div') });

function mountPanel(node: ReturnType<typeof makeNodeStatus> | null) {
  return mount(NodeInfoPanel, { props: { node }, global: { plugins: [getI18n()] } });
}

describe('NodeInfoPanel', () => {
  beforeEach(async () => {
    __resetI18nForTest();
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
  });

  it('renders nothing in the rows when node is null', () => {
    const wrapper = mountPanel(null);
    expect(wrapper.findAll('[data-test="info-value"]')).toHaveLength(0);
  });

  it('renders OS / CPU / memory / disk / agent / uptime rows', () => {
    const node = makeNodeStatus({
      identity: {
        ...makeNodeStatus().identity,
        os: 'linux',
        kernel_version: '6.1.0',
        cpu_cores: 4,
        cpu_model: 'Test CPU',
        agent_version: '1.2.3',
      },
    });
    const wrapper = mountPanel(node);
    const text = wrapper.text();
    expect(text).toContain('linux');
    expect(text).toContain('4 Core(s) · Test CPU');
    expect(text).toContain('1.2.3');
    // memory total 8 GB, uptime 90000s → 1d 1h 0m
    expect(text).toContain('7.5 GB');
    expect(text).toContain('1d 1h 0m');
  });

  it('shows n/a when memory total is missing', () => {
    const node = makeNodeStatus({ snapshot: null });
    const wrapper = mountPanel(node);
    expect(wrapper.text()).toContain('n/a');
  });
});
