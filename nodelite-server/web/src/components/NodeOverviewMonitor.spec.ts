import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import type { HistoryPoint } from '@/api';
import NodeOverviewMonitor from './NodeOverviewMonitor.vue';

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
    'node.history_window': 'History Window',
    'node.cpu_usage': 'CPU Usage',
    'node.memory_usage': 'Memory Usage',
    'node.network_traffic': 'Network Traffic',
    'node.disk_usage': 'Disk Usage',
    'node.load': 'Load',
    'node.latency_history': 'RTT',
    'node.chart.average': 'Avg {value}',
    'node.chart.zoom': 'Open enlarged chart',
    'node.clip.on': 'Clip Spikes: On',
    'node.clip.off': 'Clip Spikes: Off',
    'node.preset.last_3h': '3h',
    'node.preset.last_24h': '24h',
    'node.preset.last_3d': '3d',
    'node.preset.last_7d': '7d',
    'node.preset.last_14d': '14d',
    'node.waiting_history': 'Waiting…',
    'index.node.download': 'Down',
    'index.node.upload': 'Up',
    'common.unknown': 'Unknown',
    'common.unknown_os': 'unknown os',
    'common.not_available': 'n/a',
  },
  'zh-CN': { 'node.cpu_usage': 'CPU 使用率' },
};

const Stub = defineComponent({ render: () => h('div') });

function hp(recorded_at: string, over: Partial<HistoryPoint> = {}): HistoryPoint {
  return {
    node_id: 'n',
    recorded_at,
    cpu_usage_percent: 10,
    load_one: 0.1,
    load_five: 0.2,
    load_fifteen: 0.3,
    memory_used_percent: 20,
    rx_bytes_per_sec: 100,
    tx_bytes_per_sec: 50,
    latency_ms: 5,
    packet_loss_percent: 0.2,
    disk_used_percent: 40,
    ...over,
  };
}

function mountOverview() {
  return mount(NodeOverviewMonitor, {
    props: {
      node: makeNodeStatus(),
      history: [
        hp('2026-05-29T00:00:00Z'),
        hp('2026-05-29T00:05:00Z', { cpu_usage_percent: 30, load_one: 0.4 }),
      ],
      activeKey: 'last_24h' as const,
    },
    global: { plugins: [getI18n()] },
  });
}

function mountOverviewWithHistory(history: HistoryPoint[]) {
  return mount(NodeOverviewMonitor, {
    props: {
      node: makeNodeStatus(),
      history,
      activeKey: 'last_24h' as const,
    },
    global: { plugins: [getI18n()] },
  });
}

describe('NodeOverviewMonitor', () => {
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

  it('renders the server info band, five summary cards, and six charts', () => {
    const wrapper = mountOverview();
    expect(wrapper.find('[data-test="overview-info-band"]').exists()).toBe(true);
    expect(wrapper.findAll('.summary-card')).toHaveLength(5);
    expect(wrapper.findAll('.chart-card')).toHaveLength(6);
    expect(wrapper.find('[data-test="summary-load"]').text()).toContain('0.30');
  });

  it('uses the current snapshot when history is empty', () => {
    const wrapper = mountOverviewWithHistory([]);
    expect(wrapper.findAll('[data-test="metric-chart-empty"]')).toHaveLength(0);
    expect(wrapper.findAll('[data-test="metric-chart-svg"]')).toHaveLength(6);
  });

  it('renders preset buttons with the active one marked', () => {
    const wrapper = mountOverview();
    expect(wrapper.findAll('.preset-button')).toHaveLength(5);
    expect(wrapper.find('[data-test="preset-last_24h"]').classes()).toContain('active');
  });

  it('emits selectPreset when a preset is clicked', async () => {
    const wrapper = mountOverview();
    await wrapper.find('[data-test="preset-last_7d"]').trigger('click');
    expect(wrapper.emitted('selectPreset')?.[0]).toEqual(['last_7d']);
  });

  it('emits zoom with the metric and clipping state', async () => {
    const wrapper = mountOverview();
    await wrapper.find('[data-test="zoom-load"]').trigger('click');
    expect(wrapper.emitted('zoom')?.[0]).toEqual(['load', true]);
  });

  it('keeps per-chart spike clipping toggles enabled by default', async () => {
    const wrapper = mountOverview();
    const toggles = wrapper.findAll('.clip-toggle');
    expect(toggles).toHaveLength(6);
    for (const toggle of toggles) {
      expect(toggle.classes()).toContain('active');
      expect(toggle.attributes('aria-pressed')).toBe('true');
    }

    await wrapper.find('[data-test="clip-disk"]').trigger('click');
    expect(wrapper.find('[data-test="clip-disk"]').classes()).not.toContain('active');
    expect(wrapper.find('[data-test="clip-disk"]').attributes('aria-pressed')).toBe('false');

    await wrapper.find('[data-test="zoom-disk"]').trigger('click');
    expect(wrapper.emitted('zoom')?.at(-1)).toEqual(['disk', false]);
  });
});
