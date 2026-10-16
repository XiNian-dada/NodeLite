import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import type { HistoryPoint } from '@/api';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import { __resetI18nForTest, getI18n, setupI18n } from '@/i18n';
import NodeNetworkPanel from './NodeNetworkPanel.vue';

const FAKE_DICT = {
  en: {
    'common.online': 'Online',
    'common.offline': 'Offline',
    'index.node.download': 'Down',
    'index.node.upload': 'Up',
    'node.network_traffic': 'Network Traffic',
    'node.network.live': 'Live',
    'node.network.quality': 'Quality',
    'node.network.link_health': 'Link Health',
    'node.network.packet_loss': 'Packet Loss',
    'node.network.loss_history': 'Loss History',
    'node.network.rtt': 'RTT',
    'node.network.status': 'Status',
    'node.network.avg_rtt': 'Avg RTT',
    'node.network.peak_rate': 'Peak Rate',
    'node.network.samples': 'Samples',
    'node.network.samples_count': '{count} samples',
    'node.network.received': 'Received',
    'node.network.transmitted': 'Transmitted',
    'node.network.total_traffic': 'Total Traffic',
    'node.network.active_rate': 'Active Rate',
    'node.network.totals': 'Totals',
    'node.network.traffic_mix': 'Traffic Mix',
    'node.network.total_value': 'Total {value}',
    'node.network.avg_empty': 'Avg —',
    'node.network.avg_value': 'Avg {value}',
    'node.waiting_history': 'Waiting…',
  },
  'zh-CN': { 'common.online': '在线' },
};

const Stub = defineComponent({ render: () => h('div') });

function hp(recorded_at: string, over: Partial<HistoryPoint> = {}): HistoryPoint {
  return {
    node_id: 'n',
    recorded_at,
    cpu_usage_percent: null,
    load_one: null,
    load_five: null,
    load_fifteen: null,
    memory_used_percent: 0,
    rx_bytes_per_sec: 125_000,
    tx_bytes_per_sec: 50_000,
    latency_ms: 16,
    packet_loss_percent: 0.4,
    disk_used_percent: null,
    ...over,
  };
}

function mountPanel(
  node = makeNodeStatus(),
  history: HistoryPoint[] = [hp('2026-05-29T00:00:00Z')],
) {
  return mount(NodeNetworkPanel, {
    props: { node, history },
    global: { plugins: [getI18n()] },
  });
}

describe('NodeNetworkPanel', () => {
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

  it('renders live traffic, quality, totals, and packet loss sections', () => {
    const wrapper = mountPanel(
      makeNodeStatus({
        latency_ms: 18,
        snapshot: {
          ...makeNodeStatus().snapshot!,
          network: {
            total_rx_bytes: 1_000_000,
            total_tx_bytes: 500_000,
            rx_bytes_per_sec: 125_000,
            tx_bytes_per_sec: 50_000,
            packet_loss_percent: 0.2,
          },
        },
      }),
      [
        hp('2026-05-29T00:00:00Z', { packet_loss_percent: 0.2 }),
        hp('2026-05-29T00:01:00Z', { packet_loss_percent: 0.6, latency_ms: 24 }),
      ],
    );

    expect(wrapper.find('[data-test="network-stat-download"]').text()).toContain('Mbps');
    expect(wrapper.find('[data-test="network-stat-loss"]').text()).toContain('0.2%');
    expect(wrapper.find('[data-test="network-traffic-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="network-quality-card"]').text()).toContain('Packet Loss');
    expect(wrapper.find('[data-test="network-loss-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="network-totals-card"]').text()).toContain('Total Traffic');
    expect(wrapper.text()).not.toContain('Protocol');
  });

  it('keeps the layout present when network metrics are unavailable', () => {
    const wrapper = mountPanel(makeNodeStatus({ snapshot: null }), []);

    expect(wrapper.find('[data-test="network-stat-download"]').text()).toContain('—');
    expect(wrapper.find('[data-test="network-stat-loss"]').text()).toContain('—');
    expect(wrapper.find('[data-test="network-loss-card"]').exists()).toBe(true);
  });

  it('exposes a time navigator and emits the selected range separately from live metrics', async () => {
    const wrapper = mountPanel();
    expect(wrapper.find('[data-test="chart-range-selector"]').exists()).toBe(true);
    await wrapper.find('[data-test="range-live"]').trigger('click');
    expect(wrapper.emitted('rangeChange')?.[0]).toEqual([null]);
  });
});
