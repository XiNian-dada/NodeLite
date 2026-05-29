/**
 * Chart value formatting, ported from assets/node.html:2121-2128. Pure.
 * Distinct from lib/format.ts: this formats a single charted value by metric
 * kind and always returns a display string ("—" for non-finite).
 */

import { fmtRate } from '@/lib/format';

export type ChartValueKind = 'percent' | 'rate' | 'latency' | 'number';

export function formatChartValue(value: number | null | undefined, kind: ChartValueKind): string {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return '—';
  if (kind === 'percent') {
    return `${numeric >= 10 ? numeric.toFixed(0) : numeric.toFixed(1)}%`;
  }
  if (kind === 'rate') {
    return fmtRate(numeric) ?? '—';
  }
  if (kind === 'latency') {
    return `${numeric.toFixed(numeric >= 10 ? 0 : 1)} ms`;
  }
  return numeric >= 100 ? numeric.toFixed(0) : numeric.toFixed(1);
}
