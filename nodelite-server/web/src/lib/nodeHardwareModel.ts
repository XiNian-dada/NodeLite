import type { DiskUsage, NodeStatus } from '@/api';
import { totalDiskBytes, uniqueDisks, usedDiskBytes } from '@/lib/disks';
import { fmtBytes, fmtLatency, uptimeParts } from '@/lib/format';

export type HardwareTranslate = (key: string, named?: Record<string, number | string>) => string;
export type HardwareSeverity = 'ok' | 'warn' | 'bad';

export interface HardwareSpecRow {
  label: string;
  value: string;
}

export interface HardwareFilesystemRow {
  name: string;
  count: number;
  total: string;
  pct: number;
}

export interface HardwareSummaryCard {
  key: 'cpu' | 'memory' | 'swap' | 'load' | 'latency';
  label: string;
  value: string;
  sub: string;
  tone: 'blue' | 'green' | 'red' | 'neutral';
  bar: number | null;
}

export interface HardwareDiskRow {
  device: string;
  mount: string;
  fs: string;
  pct: number;
  bar: number;
  capacity: string;
  severity: HardwareSeverity;
}

export interface HardwareHealthRow {
  label: string;
  value: string;
  state: HardwareSeverity;
}

export interface NodeHardwareModel {
  disks: DiskUsage[];
  totalDiskText: string;
  usedDiskText: string;
  availableDiskText: string;
  diskPercent: number | null;
  diskPercentText: string;
  diskPercentBar: number;
  filesystemRows: HardwareFilesystemRow[];
  specRows: HardwareSpecRow[];
  summaryCards: HardwareSummaryCard[];
  diskRows: HardwareDiskRow[];
  healthRows: HardwareHealthRow[];
}

function fallback(value: string | null | undefined, t: HardwareTranslate): string {
  return value || t('common.not_available');
}

export function percentText(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(Number(value))) return '—';
  return `${Math.round(Number(value))}%`;
}

export function clampPercent(value: number | null | undefined): number {
  if (value == null || !Number.isFinite(Number(value))) return 0;
  return Math.max(0, Math.min(100, Number(value)));
}

function uptimeText(seconds: number | null | undefined, t: HardwareTranslate): string {
  const parts = uptimeParts(seconds);
  if (!parts) return t('common.not_available');
  const named = { days: parts.days, hours: parts.hours, minutes: parts.minutes };
  if (parts.days > 0) return t('node.uptime.days_hours', named);
  if (parts.hours > 0) return t('node.uptime.hours_minutes', named);
  return t('node.uptime.minutes', named);
}

export function severity(value: number | null | undefined): HardwareSeverity {
  if (value == null || !Number.isFinite(Number(value))) return 'ok';
  if (value >= 90) return 'bad';
  if (value >= 75) return 'warn';
  return 'ok';
}

function diskCapacity(disk: DiskUsage): string {
  return `${fmtBytes(disk.used_bytes) ?? '—'} / ${fmtBytes(disk.total_bytes) ?? '—'}`;
}

function filesystemRows(
  disks: DiskUsage[],
  totalDisk: number,
  t: HardwareTranslate,
): HardwareFilesystemRow[] {
  const totals = new Map<string, { total: number; used: number; count: number }>();
  for (const disk of disks) {
    const key = disk.fs_type || t('common.unknown');
    const current = totals.get(key) ?? { total: 0, used: 0, count: 0 };
    current.total += disk.total_bytes || 0;
    current.used += disk.used_bytes || 0;
    current.count += 1;
    totals.set(key, current);
  }
  return [...totals.entries()].map(([name, row]) => ({
    name,
    count: row.count,
    total: fmtBytes(row.total) ?? '—',
    pct: totalDisk ? (row.total / totalDisk) * 100 : 0,
  }));
}

export function buildNodeHardwareModel(
  node: NodeStatus | null,
  t: HardwareTranslate,
): NodeHardwareModel {
  const snapshot = node?.snapshot ?? null;
  const identity = node?.identity ?? null;
  const disks = uniqueDisks(snapshot?.disks);
  const totalDisk = totalDiskBytes(disks);
  const usedDisk = usedDiskBytes(disks);
  const availableDisk = Math.max(0, totalDisk - usedDisk);
  const diskPercent = totalDisk ? (usedDisk / totalDisk) * 100 : null;
  const memory = snapshot?.memory;
  const memoryPercent = memory?.total_bytes ? (memory.used_bytes / memory.total_bytes) * 100 : null;
  const swapPercent = memory?.swap_total_bytes
    ? (memory.swap_used_bytes / memory.swap_total_bytes) * 100
    : null;

  return {
    disks,
    totalDiskText: fmtBytes(totalDisk) ?? '—',
    usedDiskText: fmtBytes(usedDisk) ?? '—',
    availableDiskText: fmtBytes(availableDisk) ?? '—',
    diskPercent,
    diskPercentText: percentText(diskPercent),
    diskPercentBar: clampPercent(diskPercent),
    filesystemRows: filesystemRows(disks, totalDisk, t),
    specRows: [
      { label: t('node.info.os'), value: identity?.os || t('common.unknown_os') },
      { label: t('node.info.kernel'), value: fallback(identity?.kernel_version, t) },
      {
        label: t('node.info.cpu'),
        value: identity?.cpu_model
          ? `${identity.cpu_cores} ${t('node.hardware.cores')} · ${identity.cpu_model}`
          : t('common.unknown'),
      },
      {
        label: t('node.info.memory'),
        value: fmtBytes(memory?.total_bytes) ?? t('common.not_available'),
      },
      { label: t('node.info.virtualization'), value: fallback(identity?.agent_version, t) },
      { label: t('node.info.uptime'), value: uptimeText(snapshot?.uptime_secs, t) },
    ],
    summaryCards: [
      {
        key: 'cpu',
        label: t('node.stats.cpu'),
        value: identity?.cpu_model ?? t('common.unknown'),
        sub: `${identity?.cpu_cores ?? 0} ${t('node.hardware.cores')} · ${percentText(snapshot?.cpu_usage_percent)}`,
        tone: 'blue',
        bar: clampPercent(snapshot?.cpu_usage_percent),
      },
      {
        key: 'memory',
        label: t('node.stats.memory'),
        value: fmtBytes(memory?.total_bytes) ?? '—',
        sub: `${fmtBytes(memory?.used_bytes) ?? '—'} ${t('node.hardware.used')} · ${percentText(memoryPercent)}`,
        tone: 'green',
        bar: clampPercent(memoryPercent),
      },
      {
        key: 'swap',
        label: t('node.stats.swap'),
        value: fmtBytes(memory?.swap_total_bytes) ?? '—',
        sub: memory?.swap_total_bytes
          ? `${fmtBytes(memory.swap_used_bytes) ?? '—'} ${t('node.hardware.used')} · ${percentText(swapPercent)}`
          : t('common.not_available'),
        tone: 'neutral',
        bar: clampPercent(swapPercent),
      },
      {
        key: 'load',
        label: t('node.stats.load'),
        value: snapshot
          ? `${snapshot.load.one.toFixed(2)} / ${snapshot.load.five.toFixed(2)} / ${snapshot.load.fifteen.toFixed(2)}`
          : '—',
        sub: t('node.hardware.load_hint'),
        tone: 'neutral',
        bar: null,
      },
      {
        key: 'latency',
        label: t('node.stats.latency'),
        value: fmtLatency(node?.latency_ms) ?? '—',
        sub: node?.online ? t('common.online') : t('common.offline'),
        tone: node?.online ? 'green' : 'red',
        bar: null,
      },
    ],
    diskRows: disks.map((disk) => ({
      device: disk.device,
      mount: disk.mount_point || '—',
      fs: disk.fs_type || '—',
      pct: disk.used_percent ?? 0,
      bar: clampPercent(disk.used_percent),
      capacity: diskCapacity(disk),
      severity: severity(disk.used_percent),
    })),
    healthRows: [
      {
        label: t('node.hardware.health.status'),
        value: node?.online ? t('common.online') : t('common.offline'),
        state: node?.online ? 'ok' : 'bad',
      },
      {
        label: t('node.cpu_usage'),
        value: percentText(snapshot?.cpu_usage_percent),
        state: severity(snapshot?.cpu_usage_percent),
      },
      {
        label: t('node.memory_usage'),
        value: percentText(memoryPercent),
        state: severity(memoryPercent),
      },
      {
        label: t('node.disk_usage'),
        value: percentText(diskPercent),
        state: severity(diskPercent),
      },
      {
        label: t('node.stats.latency'),
        value: fmtLatency(node?.latency_ms) ?? '—',
        state: node?.latency_ms != null && node.latency_ms > 200 ? 'warn' : 'ok',
      },
      {
        label: t('node.hardware.partitions'),
        value: `${disks.length}`,
        state: 'ok',
      },
    ],
  };
}
