import type { HistoryPoint, NodeStatus } from '@/api';
import { averageValue, buildChartData, type ChartData, type ChartPoint } from '@/lib/chart/chartData';
import { networkSeries } from '@/lib/chart/svgModel';
import { fmtBytes, fmtLatency, fmtPercent, fmtRate } from '@/lib/format';

export type NetworkTranslate = (key: string, named?: Record<string, number | string>) => string;
export type NetworkTone = 'ok' | 'warn' | 'bad' | 'neutral';

export interface NetworkStatCard {
  key: 'download' | 'upload' | 'latency' | 'loss';
  label: string;
  value: string;
  meta: string;
  tone: NetworkTone;
}

export interface NetworkQualityRow {
  label: string;
  value: string;
  tone: NetworkTone;
}

export interface NetworkTotalRow {
  label: string;
  value: string;
}

export interface NodeNetworkModel {
  chartData: ChartData;
  trafficSeries: ReturnType<typeof networkSeries>;
  packetLoss: number | null;
  packetLossText: string;
  packetLossTone: NetworkTone;
  averagePacketLossText: string;
  totalTrafficText: string;
  rxShare: number;
  txShare: number;
  statCards: NetworkStatCard[];
  qualityRows: NetworkQualityRow[];
  totalRows: NetworkTotalRow[];
}

export function latencyTone(value: number | null | undefined): NetworkTone {
  if (value == null || !Number.isFinite(Number(value))) return 'neutral';
  if (value >= 300) return 'bad';
  if (value >= 180) return 'warn';
  return 'ok';
}

export function lossTone(value: number | null | undefined): NetworkTone {
  if (value == null || !Number.isFinite(Number(value))) return 'neutral';
  if (value >= 5) return 'bad';
  if (value >= 1) return 'warn';
  return 'ok';
}

function maxPointValue(points: ChartPoint[]): number | null {
  const values = points
    .map((point) => point.value)
    .filter((value): value is number => value != null && Number.isFinite(Number(value)));
  if (values.length === 0) return null;
  return Math.max(...values);
}

export function buildNodeNetworkModel(
  node: NodeStatus | null,
  history: HistoryPoint[],
  t: NetworkTranslate,
): NodeNetworkModel {
  const chartData = buildChartData(history);
  const network = node?.snapshot?.network ?? null;
  const rxRate = network?.rx_bytes_per_sec ?? null;
  const txRate = network?.tx_bytes_per_sec ?? null;
  const rxTotal = network?.total_rx_bytes ?? null;
  const txTotal = network?.total_tx_bytes ?? null;
  const totalTrafficBytes = rxTotal == null && txTotal == null ? null : (rxTotal ?? 0) + (txTotal ?? 0);
  const activeRate = rxRate == null && txRate == null ? null : (rxRate ?? 0) + (txRate ?? 0);
  const packetLoss = network?.packet_loss_percent ?? null;
  const averagePacketLoss = averageValue(chartData.packetLossPts);
  const averageLatency = averageValue(chartData.rttPts);
  const peakRate = maxPointValue([...chartData.dlPts, ...chartData.upPts]);
  const rxShare =
    totalTrafficBytes && rxTotal != null ? Math.max(0, Math.min(100, (rxTotal / totalTrafficBytes) * 100)) : 0;
  const txShare = totalTrafficBytes ? 100 - rxShare : 0;

  return {
    chartData,
    trafficSeries: networkSeries(chartData, t('index.node.download'), t('index.node.upload')),
    packetLoss,
    packetLossText: fmtPercent(packetLoss) ?? '—',
    packetLossTone: lossTone(packetLoss),
    averagePacketLossText: fmtPercent(averagePacketLoss) ?? '—',
    totalTrafficText: fmtBytes(totalTrafficBytes) ?? '—',
    rxShare,
    txShare,
    statCards: [
      {
        key: 'download',
        label: t('index.node.download'),
        value: fmtRate(rxRate) ?? '—',
        meta: t('node.network.total_value', { value: fmtBytes(rxTotal) ?? '—' }),
        tone: 'ok',
      },
      {
        key: 'upload',
        label: t('index.node.upload'),
        value: fmtRate(txRate) ?? '—',
        meta: t('node.network.total_value', { value: fmtBytes(txTotal) ?? '—' }),
        tone: 'neutral',
      },
      {
        key: 'latency',
        label: t('node.network.rtt'),
        value: fmtLatency(node?.latency_ms) ?? '—',
        meta: averageLatency == null ? t('node.network.avg_empty') : (fmtLatency(averageLatency) ?? '—'),
        tone: latencyTone(node?.latency_ms),
      },
      {
        key: 'loss',
        label: t('node.network.packet_loss'),
        value: fmtPercent(packetLoss) ?? '—',
        meta:
          averagePacketLoss == null
            ? t('node.network.avg_empty')
            : t('node.network.avg_value', { value: fmtPercent(averagePacketLoss) ?? '—' }),
        tone: lossTone(packetLoss),
      },
    ],
    qualityRows: [
      {
        label: t('node.network.status'),
        value: node?.online ? t('common.online') : t('common.offline'),
        tone: node?.online ? 'ok' : 'bad',
      },
      {
        label: t('node.network.avg_rtt'),
        value: fmtLatency(averageLatency) ?? '—',
        tone: latencyTone(averageLatency),
      },
      {
        label: t('node.network.peak_rate'),
        value: fmtRate(peakRate) ?? '—',
        tone: 'neutral',
      },
      {
        label: t('node.network.samples'),
        value: t('node.network.samples_count', { count: history.length }),
        tone: 'neutral',
      },
    ],
    totalRows: [
      { label: t('node.network.received'), value: fmtBytes(rxTotal) ?? '—' },
      { label: t('node.network.transmitted'), value: fmtBytes(txTotal) ?? '—' },
      { label: t('node.network.total_traffic'), value: fmtBytes(totalTrafficBytes) ?? '—' },
      { label: t('node.network.active_rate'), value: fmtRate(activeRate) ?? '—' },
    ],
  };
}
