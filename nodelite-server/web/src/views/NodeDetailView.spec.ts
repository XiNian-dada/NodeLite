import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter, type Router } from 'vue-router';
import { createApp, defineComponent, h } from 'vue';

import NodeDetailView from './NodeDetailView.vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { apiClient } from '@/api';
import { makeNode, makeNodeStatus } from '@/api/__fixtures__/nodes';

const wsMock = vi.hoisted(() => {
  const handlers = new Map<string, Set<(message: never) => void>>();

  return {
    on: vi.fn((type: string, handler: (message: never) => void) => {
      const existing = handlers.get(type) ?? new Set<(message: never) => void>();
      existing.add(handler);
      handlers.set(type, existing);
      return () => {
        existing.delete(handler);
      };
    }),
    emit(type: string, message: unknown): void {
      for (const handler of handlers.get(type) ?? []) handler(message as never);
    },
    reset(): void {
      handlers.clear();
    },
  };
});

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      nodeStatus: vi.fn(),
      nodeHistory: vi.fn().mockResolvedValue([]),
      nodeLogs: vi.fn().mockResolvedValue([]),
    },
  };
});

vi.mock('@/ws', () => ({ useWebSocket: () => wsMock }));

const mockStatus = vi.mocked(apiClient.nodeStatus);
const mockHistory = vi.mocked(apiClient.nodeHistory);
const mockLogs = vi.mocked(apiClient.nodeLogs);

const FAKE_DICT = {
  en: {
    'node.tabs.overview': 'Overview',
    'node.tabs.monitor': 'Monitor',
    'node.tabs.network': 'Network',
    'node.tabs.hardware': 'Hardware',
    'node.tabs.logs': 'Logs',
    'node.tabs.settings': 'Settings',
    'node.meta.ip': 'IP: {ip}',
    'node.meta.uptime_days': 'Up {days}d',
    'node.meta.uptime_hours': 'Up {hours}h',
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
    'node.disk_usage': 'Disk Usage',
    'node.load': 'Load',
    'node.no_disks': 'No disk metrics.',
    'node.disk.device': 'Device',
    'node.disk.mount': 'Mount',
    'node.disk.filesystem': 'FS',
    'node.disk.usage': 'Usage',
    'node.disk.capacity': 'Capacity',
    'node.cpu_usage': 'CPU Usage',
    'node.memory_usage': 'Memory Usage',
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
    'node.latency_history': 'RTT',
    'node.mounted_disks': 'Mounted Disks',
    'node.stats.cpu': 'CPU',
    'node.stats.memory': 'Memory',
    'node.stats.swap': 'Swap',
    'node.stats.load': 'Load 1/5/15',
    'node.stats.latency': 'Latency',
    'node.chart.average': 'Avg {value}',
    'node.chart.zoom': 'Open enlarged chart',
    'node.clip.on': 'Clip Spikes: On',
    'node.clip.off': 'Clip Spikes: Off',
    'node.clip.on_short': 'Clip: On',
    'node.clip.off_short': 'Clip: Off',
    'node.waiting_history': 'Waiting…',
    'node.preset.last_3h': '3h',
    'node.preset.last_24h': '24h',
    'node.preset.last_3d': '3d',
    'node.preset.last_7d': '7d',
    'node.preset.last_14d': '14d',
    'node.hardware.system': 'System',
    'node.hardware.storage': 'Storage',
    'node.hardware.filesystems': 'Filesystem Distribution',
    'node.hardware.total': 'Total',
    'node.hardware.used': 'Used',
    'node.hardware.available': 'Available',
    'node.hardware.cores': 'cores',
    'node.hardware.load_hint': '1 / 5 / 15 minute windows',
    'node.hardware.partitions': 'Partitions',
    'node.hardware.partition_count': '{count} partitions',
    'node.hardware.health.title': 'Hardware Health',
    'node.hardware.health.summary': 'Signal Summary',
    'node.hardware.health.status': 'Node Status',
    'node.logs.empty': 'No logs.',
    'node.logs.load_failed': 'Failed: {error}',
    'node.logs.level_info': 'Info',
    'node.logs.level_warn': 'Warn',
    'node.logs.level_error': 'Error',
    'index.node.download': 'Down',
    'index.node.upload': 'Up',
    'common.unknown': 'Unknown',
    'common.unknown_os': 'unknown os',
    'common.not_available': 'n/a',
    'common.online': 'Online',
    'common.offline': 'Offline',
    'common.latency_warn': 'High latency',
    'common.language': 'Language',
    'common.theme_toggle': 'Toggle theme',
    'index.nav.overview': 'Overview',
    'index.nav.settings': 'Settings',
    'index.nav.alerts': 'Alerts',
    'index.nav.account': 'Account',
  },
  'zh-CN': { 'common.online': '在线' },
};

const Stub = defineComponent({ render: () => h('div') });
const mountedWrappers = new Set<{ unmount: () => void }>();

function makeRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', name: 'dashboard', component: Stub },
      { path: '/nodes/:id', name: 'node-detail', component: NodeDetailView },
    ],
  });
}

async function mountDetail(id = 'srv-1') {
  const pinia = createPinia();
  setActivePinia(pinia);
  const router = makeRouter();
  await router.push(`/nodes/${id}`);
  await router.isReady();
  const wrapper = mount(NodeDetailView, {
    global: { plugins: [pinia, router, getI18n()] },
  });
  mountedWrappers.add(wrapper);
  await flushPromises();
  return { wrapper, router };
}

describe('NodeDetailView', () => {
  beforeEach(async () => {
    window.localStorage.clear();
    __resetI18nForTest();
    wsMock.reset();
    mockStatus.mockResolvedValue(
      makeNodeStatus({
        identity: {
          ...makeNodeStatus().identity,
          node_id: 'srv-1',
          node_label: 'Server One',
          tags: ['ip:10.0.0.9', 'region:eu'],
        },
        online: true,
        latency_ms: 12,
      }),
    );
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
    for (const wrapper of mountedWrappers) wrapper.unmount();
    mountedWrappers.clear();
    window.localStorage.clear();
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it('loads the node status for the route id', async () => {
    await mountDetail('srv-1');
    expect(mockStatus).toHaveBeenCalledWith('srv-1');
  });

  it('renders the identity header from the loaded status', async () => {
    const { wrapper } = await mountDetail('srv-1');
    expect(wrapper.find('[data-test="node-detail-view"]').text()).toContain('Server One');
    expect(wrapper.find('[data-test="node-status-badge"]').classes()).toContain('online');
    expect(wrapper.find('[data-test="node-meta"]').text()).toContain('IP: 10.0.0.9');
    expect(wrapper.find('[data-test="node-meta"]').text()).toContain('eu');
  });

  it('annotates LAN IPs in the detail header', async () => {
    mockStatus.mockResolvedValueOnce(
      makeNodeStatus({
        identity: {
          ...makeNodeStatus().identity,
          node_id: 'srv-lan',
          node_label: 'LAN Node',
          tags: [],
        },
        remote_ip: '100.64.0.8',
        geoip_country: 'LAN',
      }),
    );

    const { wrapper } = await mountDetail('srv-lan');
    expect(wrapper.find('[data-test="node-meta"]').text()).toContain('IP: 100.64.0.8 (LAN)');
  });

  it('renders the five tabs including settings', async () => {
    const { wrapper } = await mountDetail();
    for (const tab of ['overview', 'network', 'hardware', 'logs', 'settings']) {
      expect(wrapper.find(`[data-test="tab-${tab}"]`).exists()).toBe(true);
    }
    expect(wrapper.find('[data-test="tab-monitor"]').exists()).toBe(false);
  });

  it('defaults to the overview tab and switches via the URL hash', async () => {
    const { wrapper } = await mountDetail();
    expect(wrapper.find('[data-test="node-tab-pane"]').attributes('data-pane')).toBe('overview');
    expect(wrapper.find('[data-test="tab-overview"]').classes()).toContain('active');

    await wrapper.find('[data-test="tab-network"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="node-tab-pane"]').attributes('data-pane')).toBe('network');
    expect(wrapper.find('[data-test="tab-network"]').classes()).toContain('active');
  });

  it('renders the combined overview and loads high-res monitor history', async () => {
    const { wrapper } = await mountDetail('srv-1');
    expect(wrapper.find('[data-test="node-combined-overview"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="overview-summary-cards"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="overview-monitor-charts"]').exists()).toBe(true);
    expect(mockHistory).toHaveBeenCalledWith('srv-1', { windowHours: 24, maxPoints: 720 });
  });

  it('shows the network pane on the network tab', async () => {
    const { wrapper, router } = await mountDetail('srv-1');
    mockHistory.mockClear();
    await router.replace({ hash: '#network' });
    await flushPromises();
    expect(wrapper.find('[data-test="network-pane"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="network-quality-card"]').exists()).toBe(true);
    expect(mockHistory).toHaveBeenCalledWith(
      'srv-1',
      expect.objectContaining({ windowHours: 336 }),
    );
  });

  it('shows disks on the hardware tab', async () => {
    const { wrapper, router } = await mountDetail('srv-1');
    await router.replace({ hash: '#hardware' });
    await flushPromises();
    expect(wrapper.find('[data-test="node-hardware-panel"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-disks"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="hardware-health-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-info-panel"]').exists()).toBe(false);
  });

  it('loads a new high-res history window when an overview preset is selected', async () => {
    const { wrapper } = await mountDetail('srv-1');
    mockHistory.mockClear();
    await wrapper.find('[data-test="preset-last_7d"]').trigger('click');
    await flushPromises();
    expect(mockHistory).toHaveBeenCalledWith('srv-1', { windowHours: 168, maxPoints: 720 });
  });

  it('opens the zoom modal from an overview chart and closes it', async () => {
    const { wrapper } = await mountDetail('srv-1');
    await wrapper.find('[data-test="zoom-cpu"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="chart-modal"]').exists()).toBe(true);

    await wrapper.find('[data-test="chart-modal-close"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="chart-modal"]').exists()).toBe(false);
  });

  it('closes the zoom modal when leaving the overview tab', async () => {
    const { wrapper, router } = await mountDetail('srv-1');
    await wrapper.find('[data-test="zoom-cpu"]').trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-test="chart-modal"]').exists()).toBe(true);

    await router.replace({ hash: '#network' });
    await flushPromises();
    expect(wrapper.find('[data-test="chart-modal"]').exists()).toBe(false);
  });

  it('falls back to overview for the old monitor hash', async () => {
    const { wrapper, router } = await mountDetail('srv-1');
    await router.replace({ hash: '#monitor' });
    await flushPromises();
    expect(wrapper.find('[data-test="node-tab-pane"]').attributes('data-pane')).toBe('overview');
    expect(wrapper.find('[data-test="node-combined-overview"]').exists()).toBe(true);
  });

  it('shows the log panel and loads logs on the logs tab', async () => {
    const { wrapper, router } = await mountDetail('srv-1');
    await router.replace({ hash: '#logs' });
    await flushPromises();
    expect(wrapper.find('[data-test="log-panel"]').exists()).toBe(true);
    expect(mockLogs).toHaveBeenCalledWith('srv-1', 200);
  });

  it('applies websocket node updates without polling the full status', async () => {
    vi.useFakeTimers();
    const { wrapper } = await mountDetail('srv-1');
    expect(mockStatus).toHaveBeenCalledTimes(1);

    wsMock.emit('node_upsert', {
      type: 'node_upsert',
      generated_at: '2026-06-01T12:00:00Z',
      node: makeNode({
        identity: {
          node_id: 'srv-1',
          node_label: 'Realtime Server',
          hostname: 'realtime-server',
          tags: ['region:us'],
        },
        snapshot: {
          cpu_usage_percent: 88,
          load: { one: 2.5 },
          memory: { total_bytes: 8_000_000_000, used_bytes: 4_000_000_000 },
        },
        latency_ms: 320,
        online: true,
      }),
    });
    await flushPromises();

    expect(wrapper.find('.node-title__name').text()).toBe('Realtime Server');
    expect(wrapper.find('[data-test="node-status-badge"]').classes()).toContain('latency');
    expect(wrapper.find('[data-test="summary-cpu"]').text()).toContain('88%');
    expect(wrapper.find('[data-test="summary-latency"]').text()).toContain('320 ms');

    await vi.advanceTimersByTimeAsync(5000);
    await flushPromises();
    expect(mockStatus).toHaveBeenCalledTimes(1);
  });

  it('does not poll the full status while waiting for websocket reconnects', async () => {
    vi.useFakeTimers();
    await mountDetail('srv-1');
    expect(mockStatus).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(5000);
    await flushPromises();
    expect(mockStatus).toHaveBeenCalledTimes(1);
  });

  it('shows not-found when the active node is removed over websocket', async () => {
    const { wrapper } = await mountDetail('srv-1');

    wsMock.emit('node_removed', {
      type: 'node_removed',
      generated_at: '2026-06-01T12:00:00Z',
      node_id: 'srv-1',
    });
    await flushPromises();

    expect(wrapper.find('[data-test="node-not-found"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-detail-view"]').exists()).toBe(false);
  });

  it('ignores stale websocket removal messages', async () => {
    const { wrapper } = await mountDetail('srv-1');
    wsMock.emit('initial_state', {
      type: 'initial_state',
      generated_at: '2026-06-01T12:01:00Z',
      overview: {},
      nodes: [
        makeNode({
          identity: {
            node_id: 'srv-1',
            node_label: 'Server One',
            hostname: 'server-one',
            tags: [],
          },
        }),
      ],
    });
    wsMock.emit('node_removed', {
      type: 'node_removed',
      generated_at: '2026-06-01T12:00:00Z',
      node_id: 'srv-1',
    });
    await flushPromises();

    expect(wrapper.find('[data-test="node-detail-view"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-not-found"]').exists()).toBe(false);
  });
});
