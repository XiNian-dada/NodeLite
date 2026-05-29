import { describe, expect, it } from 'vitest';
import { formatChartValue } from './format';

describe('formatChartValue', () => {
  it('returns em dash for non-finite values', () => {
    expect(formatChartValue(undefined, 'percent')).toBe('—');
    expect(formatChartValue(Number.NaN, 'number')).toBe('—');
  });

  it('treats null as 0 (Number(null)===0), matching legacy', () => {
    // Charts filter out null points before formatting, so this only affects
    // direct calls; kept faithful to node.html:2122.
    expect(formatChartValue(null, 'percent')).toBe('0.0%');
  });

  it('formats percent (0 decimals at >=10, else 1)', () => {
    expect(formatChartValue(63.7, 'percent')).toBe('64%');
    expect(formatChartValue(4.2, 'percent')).toBe('4.2%');
  });

  it('formats latency in ms (0 decimals at >=10, else 1)', () => {
    expect(formatChartValue(42.6, 'latency')).toBe('43 ms');
    expect(formatChartValue(4.2, 'latency')).toBe('4.2 ms');
  });

  it('formats rate via fmtRate (bytes/sec → bits)', () => {
    expect(formatChartValue(125_000, 'rate')).toBe('1.0 Mbps');
  });

  it('formats plain numbers (0 decimals at >=100, else 1)', () => {
    expect(formatChartValue(150, 'number')).toBe('150');
    expect(formatChartValue(1.5, 'number')).toBe('1.5');
  });
});
