import { describe, expect, it } from 'vitest';
import type { HistoryPoint } from '@/api';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import { buildNodeNetworkModel, latencyTone, lossTone } from '@/lib/nodeNetworkModel';

const labels: Record<string, string> = {
  'common.online': 'Online',
  'common.offline': 'Offline',
  'index.node.download': 'Down',
  'index.node.upload': 'Up',
  'node.network.packet_loss': 'Packet Loss',
  'node.network.rtt': 'RTT',
  'node.network.status': 'Status',
  'node.network.avg_rtt': 'Avg RTT',
  'node.network.peak_rate': 'Peak Rate',
  'node.network.samples': 'Samples',
  'node.network.received': 'Received',
  'node.network.transmitted': 'Transmitted',
  'node.network.total_traffic': 'Total Traffic',
  'node.network.active_rate': 'Active Rate',
  'node.network.avg_empty': 'Avg —',
};

function t(key: string, named?: Record<string, number | string>): string {
  if (key === 'node.network.samples_count') return `${named?.count ?? 0} samples`;
  if (key === 'node.network.total_value') return `Total ${named?.value ?? '—'}`;
  if (key === 'node.network.avg_value') return `Avg ${named?.value ?? '—'}`;
  return labels[key] ?? key;
}

function hp(recorded_at: string, over: Partial<HistoryPoint> = {}): HistoryPoint {
  return {
    node_id: 'n',
    recorded_at,
    cpu_usage_percent: null,
    load_one: null,
    load_five: null,
    load_fifteen: null,
    memory_used_percent: null,
    rx_bytes_per_sec: null,
    tx_bytes_per_sec: null,
    latency_ms: null,
    packet_loss_percent: null,
    disk_used_percent: null,
    ...over,
  };
}

describe('nodeNetworkModel', () => {
  it('uses neutral placeholders when snapshot network metrics are missing', () => {
    const model = buildNodeNetworkModel(makeNodeStatus({ snapshot: null }), [], t);

    expect(model.statCards.find((card) => card.key === 'download')?.value).toBe('—');
    expect(model.statCards.find((card) => card.key === 'upload')?.value).toBe('—');
    expect(model.statCards.find((card) => card.key === 'loss')?.value).toBe('—');
    expect(model.packetLossTone).toBe('neutral');
    expect(model.totalTrafficText).toBe('—');
    expect(model.rxShare).toBe(0);
    expect(model.txShare).toBe(0);
    expect(model.qualityRows.find((row) => row.label === 'Samples')?.value).toBe('0 samples');
  });

  it('derives rates, totals, packet loss, and traffic split from node status', () => {
    const model = buildNodeNetworkModel(
      makeNodeStatus({
        latency_ms: 220,
        snapshot: {
          ...makeNodeStatus().snapshot!,
          network: {
            total_rx_bytes: 1_000_000,
            total_tx_bytes: 3_000_000,
            rx_bytes_per_sec: 100_000,
            tx_bytes_per_sec: 300_000,
            packet_loss_percent: 1.5,
          },
        },
      }),
      [hp('2026-06-30T00:00:00Z', { latency_ms: 100, packet_loss_percent: 1 })],
      t,
    );

    expect(model.statCards.find((card) => card.key === 'download')?.value).toContain('Kbps');
    expect(model.packetLossText).toBe('1.5%');
    expect(model.packetLossTone).toBe('warn');
    expect(model.rxShare).toBe(25);
    expect(model.txShare).toBe(75);
    expect(model.qualityRows.find((row) => row.label === 'Avg RTT')?.value).toBe('100 ms');
  });

  it('builds chart-derived rows for partial history samples', () => {
    const model = buildNodeNetworkModel(
      makeNodeStatus(),
      [
        hp('2026-06-30T00:00:00Z', {
          rx_bytes_per_sec: Number.NaN,
          tx_bytes_per_sec: 50,
          latency_ms: null,
          packet_loss_percent: null,
        }),
        hp('2026-06-30T00:01:00Z', {
          rx_bytes_per_sec: 200,
          tx_bytes_per_sec: 25,
          latency_ms: 42,
          packet_loss_percent: 0.5,
        }),
      ],
      t,
    );

    expect(model.qualityRows.find((row) => row.label === 'Peak Rate')?.value).toContain('bps');
    expect(model.qualityRows.find((row) => row.label === 'Avg RTT')?.value).toBe('21 ms');
    expect(model.averagePacketLossText).toBe('0.3%');
  });

  it('classifies latency and loss tones at thresholds', () => {
    expect(latencyTone(null)).toBe('neutral');
    expect(latencyTone(179)).toBe('ok');
    expect(latencyTone(180)).toBe('warn');
    expect(latencyTone(300)).toBe('bad');
    expect(lossTone(undefined)).toBe('neutral');
    expect(lossTone(0.9)).toBe('ok');
    expect(lossTone(1)).toBe('warn');
    expect(lossTone(5)).toBe('bad');
  });
});
