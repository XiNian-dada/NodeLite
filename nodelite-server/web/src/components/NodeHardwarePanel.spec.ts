import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import type { DiskUsage } from '@/api';
import NodeHardwarePanel from './NodeHardwarePanel.vue';

const FAKE_DICT = {
  en: {
    'common.not_available': 'n/a',
    'common.unknown': 'Unknown',
    'common.unknown_os': 'unknown os',
    'common.online': 'Online',
    'common.offline': 'Offline',
    'node.info.title': 'Server Info',
    'node.info.os': 'OS',
    'node.info.kernel': 'Kernel',
    'node.info.cpu': 'CPU',
    'node.info.memory': 'Memory',
    'node.info.virtualization': 'Agent',
    'node.info.uptime': 'Uptime',
    'node.uptime.days_hours': '{days}d {hours}h {minutes}m',
    'node.uptime.hours_minutes': '{hours}h {minutes}m',
    'node.uptime.minutes': '{minutes}m',
    'node.stats.cpu': 'CPU',
    'node.stats.memory': 'Memory',
    'node.stats.swap': 'Swap',
    'node.stats.load': 'Load 1/5/15',
    'node.stats.latency': 'Latency',
    'node.cpu_usage': 'CPU Usage',
    'node.memory_usage': 'Memory Usage',
    'node.disk_usage': 'Disk Usage',
    'node.load': 'Load',
    'node.mounted_disks': 'Mounted Disks',
    'node.no_disks': 'No disk metrics.',
    'node.disk.device': 'Device',
    'node.disk.mount': 'Mount',
    'node.disk.filesystem': 'Filesystem',
    'node.disk.usage': 'Usage',
    'node.disk.capacity': 'Capacity',
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
  },
  'zh-CN': { 'common.online': '在线' },
};

const Stub = defineComponent({ render: () => h('div') });

function disk(over: Partial<DiskUsage>): DiskUsage {
  return {
    device: '/dev/sda1',
    mount_point: '/',
    fs_type: 'ext4',
    total_bytes: 100_000_000_000,
    available_bytes: 60_000_000_000,
    used_bytes: 40_000_000_000,
    used_percent: 40,
    ...over,
  };
}

function mountHardware(node = makeNodeStatus()) {
  return mount(NodeHardwarePanel, { props: { node }, global: { plugins: [getI18n()] } });
}

describe('NodeHardwarePanel', () => {
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

  it('renders system, storage, filesystem, summary, disk, and health sections', () => {
    const wrapper = mountHardware();
    expect(wrapper.find('[data-test="hardware-spec-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="hardware-storage-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="hardware-filesystem-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="hardware-summary-cards"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="node-disks"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="hardware-health-card"]').exists()).toBe(true);
  });

  it('deduplicates disks and shows filesystem distribution', () => {
    const node = makeNodeStatus();
    node.snapshot!.disks = [
      disk({ device: '/dev/sda1', total_bytes: 100, used_bytes: 40, used_percent: 40 }),
      disk({ device: '/dev/sda1', total_bytes: 100, used_bytes: 40, used_percent: 40 }),
      disk({
        device: '/dev/sdb1',
        mount_point: '/data',
        fs_type: 'xfs',
        total_bytes: 200,
        used_bytes: 190,
        used_percent: 95,
      }),
    ];

    const wrapper = mountHardware(node);
    expect(wrapper.findAll('[data-test="disk-row"]')).toHaveLength(2);
    expect(wrapper.text()).toContain('ext4');
    expect(wrapper.text()).toContain('xfs');
    expect(wrapper.find('.usage-track .bad').exists()).toBe(true);
  });

  it('keeps long mount paths inside their grid cell and exposes the full value', () => {
    const node = makeNodeStatus();
    const mountPoint =
      '/private/var/folders/y8/a-very-long-temporary-volume-name/that-must-not-overlap';
    node.snapshot!.disks = [disk({ mount_point: mountPoint })];

    const wrapper = mountHardware(node);
    const mount = wrapper.get('[data-test="disk-mount"]');
    expect(mount.classes()).toContain('mount');
    expect(mount.attributes('title')).toBe(mountPoint);
    expect(mount.text()).toBe(mountPoint);
  });

  it('renders an empty disk placeholder without temperature fields', () => {
    const wrapper = mountHardware(makeNodeStatus({ snapshot: null }));
    expect(wrapper.find('[data-test="node-disks-empty"]').exists()).toBe(true);
    expect(wrapper.text().toLowerCase()).not.toContain('temperature');
  });
});
