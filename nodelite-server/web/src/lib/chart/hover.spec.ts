import { describe, expect, it } from 'vitest';
import type { ChartPoint } from './chartData';
import { buildAreaChart, buildMultiAreaChart } from './svgModel';
import { computeHover, nearestByKey } from './hover';
import type { HoverPoint } from './svgModel';

function hoverPts(xs: number[]): HoverPoint[] {
  return xs.map((x) => ({ x, y: 0, value: x, ts: x * 1000 }));
}

function pts(values: number[]): ChartPoint[] {
  return values.map((value, i) => ({ ts: i * 60_000, value }));
}

describe('nearestByKey', () => {
  const points = hoverPts([0, 10, 20, 30]);

  it('returns null for empty', () => {
    expect(nearestByKey([], 5, 'x')).toBeNull();
  });

  it('finds the nearest by x', () => {
    expect(nearestByKey(points, 12, 'x')!.x).toBe(10);
    expect(nearestByKey(points, 16, 'x')!.x).toBe(20);
    expect(nearestByKey(points, 100, 'x')!.x).toBe(30);
    expect(nearestByKey(points, -5, 'x')!.x).toBe(0);
  });

  it('finds the nearest by ts', () => {
    // ts = x*1000
    expect(nearestByKey(points, 9000, 'ts')!.ts).toBe(10_000);
  });
});

describe('computeHover', () => {
  const model = buildAreaChart(pts([10, 50, 90]), {
    width: 600,
    height: 200,
    valueKind: 'percent',
    color: 'var(--chart-cpu)',
    label: 'CPU',
  });

  it('returns null for a degenerate rect', () => {
    expect(computeHover(model, 100, 'x', { width: 0, height: 0 }, () => '')).toBeNull();
  });

  it('places the crosshair at the nearest point and spans the plot', () => {
    const anchorX = model.series[0]!.points[1]!.x;
    const hover = computeHover(model, anchorX + 2, 'x', { width: 600, height: 200 }, () => 't');
    expect(hover).not.toBeNull();
    expect(hover!.lineX).toBeCloseTo(anchorX, 5);
    expect(hover!.lineY1).toBe(model.padTop);
    expect(hover!.lineY2).toBe(model.height - model.padBottom);
    expect(hover!.circles).toHaveLength(1);
    expect(hover!.tooltip.rows[0]).toMatchObject({ label: 'CPU', color: 'var(--chart-cpu)' });
    expect(hover!.tooltip.rows[0]!.value).toContain('%');
  });

  it('builds one circle + row per series for multi-series', () => {
    const multi = buildMultiAreaChart(
      [
        { label: 'down', color: 'cdown', points: pts([100, 200]) },
        { label: 'up', color: 'cup', points: pts([10, 20]) },
      ],
      { width: 600, height: 220, valueKind: 'rate' },
    );
    const hover = computeHover(multi, multi.series[0]!.points[0]!.x, 'x', { width: 600, height: 220 }, () => 't');
    expect(hover!.circles).toHaveLength(2);
    expect(hover!.tooltip.rows.map((r) => r.label)).toEqual(['down', 'up']);
  });

  it('uses formatTime for the tooltip header from the anchor ts', () => {
    const anchorX = model.series[0]!.points[0]!.x;
    const hover = computeHover(model, anchorX, 'x', { width: 600, height: 200 }, (ts) => `@${ts}`);
    expect(hover!.tooltip.time).toBe('@0');
  });

  it('flips the tooltip transform based on vertical position', () => {
    // Anchor at the max value sits near the top → tooltip drops below (+14px).
    const topModel = buildAreaChart(pts([0, 100]), {
      width: 600,
      height: 200,
      valueKind: 'percent',
      color: 'c',
    });
    const topPoint = topModel.series[0]!.points[1]!; // value 100 → y at padTop
    const hover = computeHover(topModel, topPoint.x, 'x', { width: 600, height: 200 }, () => 't');
    expect(hover!.tooltip.transform).toContain('14px');
  });
});
