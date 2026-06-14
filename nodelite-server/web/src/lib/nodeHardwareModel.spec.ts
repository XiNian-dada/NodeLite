import { describe, expect, it } from 'vitest';
import type { DiskUsage } from '@/api';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import {
  buildNodeHardwareModel,
  clampPercent,
  percentText,
  severity,
  type HardwareTranslate,
} from './nodeHardwareModel';

const labels: Record<string, string> = {
  'common.not_available': 'n/a',
  'common.unknown': 'Unknown',
  'common.unknown_os': 'unknown os',
  'common.online': 'Online',
  'common.offline': 'Offline',
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
  'node.hardware.used': 'Used',
  'node.hardware.cores': 'cores',
  'node.hardware.load_hint': '1 / 5 / 15 minute windows',
  'node.hardware.health.status': 'Node Status',
  'node.hardware.partitions': 'Partitions',
};

const t: HardwareTranslate = (key, named) => {
  const value = labels[key] ?? key;
  return value.replace(/\{(\w+)\}/g, (_, name: string) => String(named?.[name] ?? ''));
};

function disk(over: Partial<DiskUsage>): DiskUsage {
  return {
    device: '/dev/sda1',
    mount_point: '/',
    fs_type: 'ext4',
    total_bytes: 100,
    available_bytes: 60,
    used_bytes: 40,
    used_percent: 40,
    ...over,
  };
}

describe('nodeHardwareModel', () => {
  it('formats and clamps percent values', () => {
    expect(percentText(null)).toBe('—');
    expect(percentText(Number.NaN)).toBe('—');
    expect(percentText(74.6)).toBe('75%');
    expect(clampPercent(null)).toBe(0);
    expect(clampPercent(-10)).toBe(0);
    expect(clampPercent(140)).toBe(100);
  });

  it('classifies health severity boundaries', () => {
    expect(severity(null)).toBe('ok');
    expect(severity(74.9)).toBe('ok');
    expect(severity(75)).toBe('warn');
    expect(severity(90)).toBe('bad');
  });

  it('deduplicates disks and aggregates filesystem rows', () => {
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

    const model = buildNodeHardwareModel(node, t);

    expect(model.disks).toHaveLength(2);
    expect(model.diskPercent).toBeCloseTo(76.666, 2);
    expect(model.diskPercentBar).toBeCloseTo(76.666, 2);
    expect(model.filesystemRows.map((row) => row.name)).toEqual(['ext4', 'xfs']);
    expect(model.filesystemRows.map((row) => row.count)).toEqual([1, 1]);
    expect(model.diskRows.find((row) => row.device === '/dev/sdb1')?.severity).toBe('bad');
  });

  it('builds empty hardware rows when the snapshot is missing', () => {
    const model = buildNodeHardwareModel(makeNodeStatus({ snapshot: null, online: false }), t);

    expect(model.disks).toHaveLength(0);
    expect(model.diskRows).toHaveLength(0);
    expect(model.filesystemRows).toHaveLength(0);
    expect(model.diskPercent).toBeNull();
    expect(model.diskPercentText).toBe('—');
    expect(model.specRows.find((row) => row.label === 'Memory')?.value).toBe('n/a');
    expect(model.healthRows[0]).toEqual({
      label: 'Node Status',
      value: 'Offline',
      state: 'bad',
    });
  });

  it('keeps memory and swap percentages bounded for zero totals', () => {
    const node = makeNodeStatus();
    node.snapshot!.memory.total_bytes = 0;
    node.snapshot!.memory.used_bytes = 10;
    node.snapshot!.memory.swap_total_bytes = 0;
    node.snapshot!.memory.swap_used_bytes = 10;

    const model = buildNodeHardwareModel(node, t);
    const memoryCard = model.summaryCards.find((card) => card.key === 'memory');
    const swapCard = model.summaryCards.find((card) => card.key === 'swap');

    expect(memoryCard?.bar).toBe(0);
    expect(memoryCard?.sub).toContain('—');
    expect(swapCard?.bar).toBe(0);
    expect(swapCard?.sub).toBe('n/a');
  });
});
