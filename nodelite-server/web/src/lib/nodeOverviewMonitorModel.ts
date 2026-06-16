import type { HistoryPoint, NodeStatus } from '@/api';
import {
  averageValue,
  buildChartData,
  type ChartData,
  type ChartPoint,
} from '@/lib/chart/chartData';
import { formatChartValue, type ChartValueKind } from '@/lib/chart/format';
import { loadSeries, networkSeries, type MultiSeriesInput } from '@/lib/chart/svgModel';
import { totalDiskBytes, uniqueDisks, usedDiskBytes } from '@/lib/disks';
import { fmtBytes, fmtRate, uptimeParts } from '@/lib/format';

export type OverviewMonitorMetric = 'cpu' | 'memory' | 'network' | 'load' | 'disk' | 'latency';

export type OverviewMonitorTranslate = (
  key: string,
  named?: Record<string, number | string>,
) => string;

export type ClipState = Record<OverviewMonitorMetric, boolean>;

export interface OverviewInfoRow {
  label: string;
  value: string;
}

export interface OverviewSummaryCard {
  key: OverviewMonitorMetric;
  label: string;
  value: string;
  sub: string;
  progress: number | null;
  tone: 'green' | 'blue' | 'teal' | 'yellow' | 'neutral';
}

export interface OverviewChartProps {
  points?: ChartPoint[];
  series?: MultiSeriesInput[];
  valueKind: ChartValueKind;
  color?: string;
  label?: string;
  maxValue?: number;
  clipSpikes: boolean;
}

export interface OverviewChartModel {
  metric: OverviewMonitorMetric;
  title: string;
  meta: string;
  sub: string;
  chartProps: OverviewChartProps;
}

export interface OverviewMonitorModel {
  effectiveHistory: HistoryPoint[];
  data: ChartData;
  infoRows: OverviewInfoRow[];
  summaryCards: OverviewSummaryCard[];
  charts: OverviewChartModel[];
}

export function currentHistoryPoint(node: NodeStatus | null): HistoryPoint | null {
  const snapshot = node?.snapshot;
  if (!node || !snapshot) return null;
  const disks = uniqueDisks(snapshot.disks);
  const totalDisk = totalDiskBytes(disks);
  const usedDisk = usedDiskBytes(disks);
  const memoryTotal = snapshot.memory.total_bytes;
  return {
    node_id: node.identity.node_id,
    recorded_at: snapshot.collected_at,
    cpu_usage_percent: snapshot.cpu_usage_percent,
    load_one: snapshot.load.one,
    load_five: snapshot.load.five,
    load_fifteen: snapshot.load.fifteen,
    memory_used_percent: memoryTotal > 0 ? (snapshot.memory.used_bytes / memoryTotal) * 100 : 0,
    rx_bytes_per_sec: snapshot.network.rx_bytes_per_sec,
    tx_bytes_per_sec: snapshot.network.tx_bytes_per_sec,
    latency_ms: node.latency_ms,
    packet_loss_percent: snapshot.network.packet_loss_percent,
    disk_used_percent: totalDisk > 0 ? (usedDisk / totalDisk) * 100 : null,
  };
}

export function effectiveHistory(history: HistoryPoint[], node: NodeStatus | null): HistoryPoint[] {
  const current = currentHistoryPoint(node);
  if (!current) return history;
  if (history.some((point) => point.recorded_at === current.recorded_at)) return history;
  return [...history, current];
}

function uptimeText(seconds: number | null | undefined, t: OverviewMonitorTranslate): string {
  const parts = uptimeParts(seconds);
  if (!parts) return t('common.not_available');
  const named = { days: parts.days, hours: parts.hours, minutes: parts.minutes };
  if (parts.days > 0) return t('node.uptime.days_hours', named);
  if (parts.hours > 0) return t('node.uptime.hours_minutes', named);
  return t('node.uptime.minutes', named);
}

export function buildOverviewInfoRows(
  node: NodeStatus | null,
  t: OverviewMonitorTranslate,
): OverviewInfoRow[] {
  if (!node) return [];
  const id = node.identity;
  const snapshot = node.snapshot;
  const disks = uniqueDisks(snapshot?.disks);
  const totalDisk = totalDiskBytes(disks);
  const cpuLine = id.cpu_cores
    ? `${t('node.info.cores', { count: id.cpu_cores })}${id.cpu_model ? ` · ${id.cpu_model}` : ''}`
    : (id.cpu_model ?? t('common.unknown'));

  return [
    { label: t('node.info.os'), value: id.os || t('common.unknown_os') },
    { label: t('node.info.kernel'), value: id.kernel_version || t('common.unknown') },
    { label: t('node.info.cpu'), value: cpuLine },
    {
      label: t('node.info.memory'),
      value: snapshot?.memory.total_bytes
        ? (fmtBytes(snapshot.memory.total_bytes) ?? t('common.not_available'))
        : t('common.not_available'),
    },
    {
      label: t('node.info.disk'),
      value: totalDisk
        ? (fmtBytes(totalDisk) ?? t('common.not_available'))
        : t('common.not_available'),
    },
    { label: t('node.info.virtualization'), value: id.agent_version || t('common.unknown') },
    { label: t('node.info.uptime'), value: uptimeText(snapshot?.uptime_secs, t) },
  ];
}

function memoryPercent(node: NodeStatus | null): number | null {
  const memory = node?.snapshot?.memory;
  return memory?.total_bytes ? (memory.used_bytes / memory.total_bytes) * 100 : null;
}

function diskSummary(node: NodeStatus | null): { pct: number | null; used: string; total: string } {
  const disks = uniqueDisks(node?.snapshot?.disks);
  const total = totalDiskBytes(disks);
  const used = usedDiskBytes(disks);
  return {
    pct: total ? (used / total) * 100 : null,
    used: total ? (fmtBytes(used) ?? '—') : '—',
    total: total ? (fmtBytes(total) ?? '—') : '—',
  };
}

function percentText(value: number | null | undefined): string {
  return value == null ? '—' : formatChartValue(value, 'percent');
}

function progress(value: number | null | undefined): number | null {
  if (value == null || !Number.isFinite(Number(value))) return null;
  return Math.max(0, Math.min(100, Number(value)));
}

function avgText(points: ChartPoint[], kind: ChartValueKind, t: OverviewMonitorTranslate): string {
  const avg = averageValue(points);
  return t('node.chart.average', {
    value: avg == null ? '—' : formatChartValue(avg, kind),
  });
}

export function buildOverviewSummaryCards(
  node: NodeStatus | null,
  data: ChartData,
  t: OverviewMonitorTranslate,
): OverviewSummaryCard[] {
  const memory = node?.snapshot?.memory;
  const memPct = memoryPercent(node);
  const disk = diskSummary(node);
  const cpuValue = node?.snapshot?.cpu_usage_percent ?? null;
  const loadValue = node?.snapshot?.load ?? null;
  const latencyValue = node?.latency_ms ?? null;

  return [
    {
      key: 'cpu',
      label: t('node.cpu_usage'),
      value: percentText(cpuValue),
      sub: avgText(data.cpuPts, 'percent', t),
      progress: progress(cpuValue),
      tone: 'green',
    },
    {
      key: 'memory',
      label: t('node.memory_usage'),
      value: percentText(memPct),
      sub: memory?.total_bytes
        ? `${fmtBytes(memory.used_bytes) ?? '—'} / ${fmtBytes(memory.total_bytes) ?? '—'}`
        : '—',
      progress: progress(memPct),
      tone: 'blue',
    },
    {
      key: 'disk',
      label: t('node.disk_usage'),
      value: percentText(disk.pct),
      sub: `${disk.used} / ${disk.total}`,
      progress: progress(disk.pct),
      tone: 'teal',
    },
    {
      key: 'load',
      label: t('node.load'),
      value: loadValue ? loadValue.one.toFixed(2) : '—',
      sub: loadValue
        ? `${loadValue.one.toFixed(2)} / ${loadValue.five.toFixed(2)} / ${loadValue.fifteen.toFixed(2)}`
        : '—',
      progress: null,
      tone: 'neutral',
    },
    {
      key: 'latency',
      label: t('node.latency_history'),
      value: latencyValue == null ? '—' : formatChartValue(latencyValue, 'latency'),
      sub: avgText(data.rttPts, 'latency', t),
      progress: null,
      tone: 'yellow',
    },
  ];
}

export function buildOverviewCharts(
  node: NodeStatus | null,
  data: ChartData,
  clipSpikes: ClipState,
  t: OverviewMonitorTranslate,
): OverviewChartModel[] {
  const memPct = memoryPercent(node);
  const disk = diskSummary(node);
  const cpuValue = node?.snapshot?.cpu_usage_percent ?? null;
  const loadValue = node?.snapshot?.load ?? null;
  const latencyValue = node?.latency_ms ?? null;
  const networkValue = node?.snapshot?.network ?? null;

  return [
    {
      metric: 'cpu',
      title: t('node.cpu_usage'),
      meta: percentText(cpuValue),
      sub: avgText(data.cpuPts, 'percent', t),
      chartProps: {
        points: data.cpuPts,
        valueKind: 'percent',
        color: 'var(--chart-cpu)',
        label: t('node.cpu_usage'),
        clipSpikes: clipSpikes.cpu,
      },
    },
    {
      metric: 'memory',
      title: t('node.memory_usage'),
      meta: percentText(memPct),
      sub: avgText(data.memPts, 'percent', t),
      chartProps: {
        points: data.memPts,
        valueKind: 'percent',
        color: 'var(--chart-memory)',
        label: t('node.memory_usage'),
        maxValue: 100,
        clipSpikes: clipSpikes.memory,
      },
    },
    {
      metric: 'network',
      title: t('node.network_traffic'),
      meta: fmtRate(networkValue?.rx_bytes_per_sec) ?? '—',
      sub: fmtRate(networkValue?.tx_bytes_per_sec) ?? '—',
      chartProps: {
        series: networkSeries(data, t('index.node.download'), t('index.node.upload')),
        valueKind: 'rate',
        clipSpikes: clipSpikes.network,
      },
    },
    {
      metric: 'load',
      title: t('node.load'),
      meta: loadValue ? loadValue.one.toFixed(2) : '—',
      sub: avgText(data.loadOnePts, 'number', t),
      chartProps: {
        series: loadSeries(data),
        valueKind: 'number',
        clipSpikes: clipSpikes.load,
      },
    },
    {
      metric: 'disk',
      title: t('node.disk_usage'),
      meta: percentText(disk.pct),
      sub: avgText(data.diskPts, 'percent', t),
      chartProps: {
        points: data.diskPts,
        valueKind: 'percent',
        color: 'var(--chart-disk)',
        label: t('node.disk_usage'),
        maxValue: 100,
        clipSpikes: clipSpikes.disk,
      },
    },
    {
      metric: 'latency',
      title: t('node.latency_history'),
      meta: latencyValue == null ? '—' : formatChartValue(latencyValue, 'latency'),
      sub: avgText(data.rttPts, 'latency', t),
      chartProps: {
        points: data.rttPts,
        valueKind: 'latency',
        color: 'var(--chart-latency)',
        label: t('node.latency_history'),
        clipSpikes: clipSpikes.latency,
      },
    },
  ];
}

export function buildOverviewMonitorModel(params: {
  node: NodeStatus | null;
  history: HistoryPoint[];
  clipSpikes: ClipState;
  t: OverviewMonitorTranslate;
}): OverviewMonitorModel {
  const history = effectiveHistory(params.history, params.node);
  const data = buildChartData(history);
  return {
    effectiveHistory: history,
    data,
    infoRows: buildOverviewInfoRows(params.node, params.t),
    summaryCards: buildOverviewSummaryCards(params.node, data, params.t),
    charts: buildOverviewCharts(params.node, data, params.clipSpikes, params.t),
  };
}
