import { describe, expect, it } from 'vitest';
import type { HistoryPoint } from '@/api';
import { makeNodeStatus } from '@/api/__fixtures__/nodes';
import {
  buildOverviewMonitorModel,
  currentHistoryPoint,
  effectiveHistory,
  type ClipState,
  type OverviewMonitorTranslate,
} from './nodeOverviewMonitorModel';

const t: OverviewMonitorTranslate = (key, named) => {
  if (key === 'node.info.cores') return `${named?.count} Core(s)`;
  if (key === 'node.chart.average') return `Avg ${named?.value}`;
  return key;
};

const clipSpikes: ClipState = {
  cpu: true,
  memory: true,
  network: true,
  load: true,
  disk: true,
  latency: true,
};

function hp(recorded_at: string, over: Partial<HistoryPoint> = {}): HistoryPoint {
  return {
    node_id: 'node-a',
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

describe('nodeOverviewMonitorModel', () => {
  it('creates a current history point from the latest snapshot', () => {
    const current = currentHistoryPoint(makeNodeStatus());

    expect(current).toMatchObject({
      node_id: 'node-a',
      recorded_at: '2026-05-29T00:00:00Z',
      cpu_usage_percent: 12.5,
      load_one: 0.3,
      load_five: 0.4,
      load_fifteen: 0.5,
      memory_used_percent: 25,
      rx_bytes_per_sec: 10,
      tx_bytes_per_sec: 20,
      latency_ms: 5,
      packet_loss_percent: 0.2,
      disk_used_percent: 40,
    });
  });

  it('does not create a current point without a node snapshot', () => {
    expect(currentHistoryPoint(null)).toBeNull();
    expect(currentHistoryPoint(makeNodeStatus({ snapshot: null }))).toBeNull();
  });

  it('appends the current point only when the timestamp is new', () => {
    const node = makeNodeStatus();
    const oldPoint = hp('2026-05-28T23:55:00Z');
    const appended = effectiveHistory([oldPoint], node);

    expect(appended).toHaveLength(2);
    expect(appended[0]).toBe(oldPoint);
    expect(appended[1]?.recorded_at).toBe('2026-05-29T00:00:00Z');

    const duplicateHistory = [hp('2026-05-29T00:00:00Z')];
    expect(effectiveHistory(duplicateHistory, node)).toBe(duplicateHistory);
  });

  it('builds chart data from the current snapshot when history is empty', () => {
    const model = buildOverviewMonitorModel({
      node: makeNodeStatus(),
      history: [],
      clipSpikes,
      t,
    });

    expect(model.effectiveHistory).toHaveLength(1);
    expect(model.data.cpuPts).toHaveLength(1);
    expect(model.infoRows).toHaveLength(7);
    expect(model.summaryCards.map((card) => card.key)).toEqual([
      'cpu',
      'memory',
      'disk',
      'load',
      'latency',
    ]);
    expect(model.charts.map((chart) => chart.metric)).toEqual([
      'cpu',
      'memory',
      'network',
      'load',
      'disk',
      'latency',
    ]);
  });

  it('keeps null disk usage and zero memory percent bounded for empty hardware data', () => {
    const node = makeNodeStatus({
      snapshot: {
        collected_at: '2026-05-29T00:00:00Z',
        cpu_usage_percent: null,
        load: { one: 0, five: 0, fifteen: 0 },
        memory: {
          total_bytes: 0,
          used_bytes: 0,
          available_bytes: 0,
          swap_total_bytes: 0,
          swap_used_bytes: 0,
        },
        uptime_secs: 0,
        disks: [],
        network: {
          total_rx_bytes: 0,
          total_tx_bytes: 0,
          rx_bytes_per_sec: null,
          tx_bytes_per_sec: null,
          packet_loss_percent: null,
        },
      },
    });
    const current = currentHistoryPoint(node);
    const model = buildOverviewMonitorModel({ node, history: [], clipSpikes, t });

    expect(current?.memory_used_percent).toBe(0);
    expect(current?.disk_used_percent).toBeNull();
    expect(model.summaryCards.find((card) => card.key === 'memory')?.progress).toBeNull();
    expect(model.summaryCards.find((card) => card.key === 'disk')?.value).toBe('—');
  });
});
