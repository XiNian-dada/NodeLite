import { describe, expect, it } from 'vitest';
import { maxThresholdFor } from './alertLimits';

describe('maxThresholdFor', () => {
  it('caps cpu, memory and disk percentages at 100', () => {
    expect(maxThresholdFor('cpu_usage_percent')).toBe(100);
    expect(maxThresholdFor('memory_usage_percent')).toBe(100);
    expect(maxThresholdFor('disk_usage_percent')).toBe(100);
  });

  it('leaves traffic usage and non-percentage metrics uncapped', () => {
    expect(maxThresholdFor('traffic_usage_percent')).toBeUndefined();
    expect(maxThresholdFor('latency_ms')).toBeUndefined();
    expect(maxThresholdFor('offline_minutes')).toBeUndefined();
  });
});
