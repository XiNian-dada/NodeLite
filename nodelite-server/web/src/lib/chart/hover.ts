/**
 * Pure hover math, extracted from installChartHover (node.html:2230-2329).
 * The imperative setAttribute/innerHTML mutation lives in useChart/MetricChart;
 * this module only computes "given a target on this chart model, where do the
 * crosshair, circles, and tooltip go".
 */

import { formatChartValue } from './format';
import type { ChartModel, HoverPoint, ChartSeriesModel } from './svgModel';

export type HoverKey = 'x' | 'ts';

function keyOf(point: HoverPoint, key: HoverKey): number {
  return Number(key === 'x' ? point.x : point.ts);
}

/** Binary-search the point nearest `target` by the chosen key. */
export function nearestByKey(
  points: HoverPoint[],
  target: number,
  key: HoverKey,
): HoverPoint | null {
  if (points.length === 0) return null;
  let lo = 0;
  let hi = points.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (keyOf(points[mid]!, key) < target) lo = mid + 1;
    else hi = mid;
  }
  const right = points[lo]!;
  const left = lo > 0 ? points[lo - 1]! : right;
  return Math.abs(keyOf(right, key) - target) < Math.abs(keyOf(left, key) - target) ? right : left;
}

export interface HoverCircle {
  cx: number;
  cy: number;
  color: string;
}

export interface HoverRow {
  label: string;
  color: string;
  value: string;
}

export interface HoverModel {
  lineX: number;
  lineY1: number;
  lineY2: number;
  circles: HoverCircle[];
  anchorTs: number | null;
  tooltip: {
    left: number;
    top: number;
    transform: string;
    time: string;
    rows: HoverRow[];
  };
}

interface Match {
  series: ChartSeriesModel;
  point: HoverPoint;
}

/**
 * Compute the crosshair + tooltip placement for a target on `model`.
 * `target` is in chart-view units of `key` (an x pixel, or a ts). rect
 * dimensions map view coords to the on-screen tooltip position.
 * `formatTime` turns an epoch-ms ts into the tooltip header (i18n/locale
 * lives in the caller). Returns null when there are no points.
 */
export function computeHover(
  model: ChartModel,
  target: number,
  key: HoverKey,
  rect: { width: number; height: number },
  formatTime: (ts: number) => string,
): HoverModel | null {
  if (rect.width <= 0 || rect.height <= 0) return null;
  const matches: Match[] = [];
  for (const series of model.series) {
    const point = nearestByKey(series.points, target, key);
    if (point) matches.push({ series, point });
  }
  if (matches.length === 0) return null;

  const anchor = matches.reduce(
    (best, m) =>
      Math.abs(keyOf(m.point, key) - target) < Math.abs(keyOf(best.point, key) - target) ? m : best,
    matches[0]!,
  ).point;

  const circles: HoverCircle[] = matches.map((m) => ({
    cx: m.point.x,
    cy: m.point.y,
    color: m.series.color,
  }));
  const rows: HoverRow[] = matches.map((m) => ({
    label: m.series.label,
    color: m.series.color,
    value: formatChartValue(m.point.value, m.series.kind),
  }));

  const left = Math.min(rect.width - 74, Math.max(74, (anchor.x / model.width) * rect.width));
  const minCy = Math.min(...matches.map((m) => m.point.y));
  const top = Math.max(10, (minCy / model.height) * rect.height);
  const transform = top < 88 ? 'translate(-50%, 14px)' : 'translate(-50%, calc(-100% - 10px))';

  return {
    lineX: anchor.x,
    lineY1: model.padTop,
    lineY2: model.height - model.padBottom,
    circles,
    anchorTs: anchor.ts,
    tooltip: {
      left,
      top,
      transform,
      time: anchor.ts != null ? formatTime(anchor.ts) : '',
      rows,
    },
  };
}
