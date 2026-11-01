import type { AlertMetric } from '@/api';

/**
 * Mirrors the server-side bounds in nodelite-proto `alert_limits.rs`. The server rejects
 * saves above these with a 400, so the inputs cap them up front.
 */
export const MAX_ALERT_WINDOW_MINUTES = 20160;
export const MAX_ALERT_COOLDOWN_MINUTES = 43200;
export const MAX_INSPECTION_LOOKBACK_HOURS = 336;
export const MAX_INSPECTION_OFFLINE_GRACE_MINUTES = MAX_ALERT_WINDOW_MINUTES;

// Traffic usage is measured against a quota, so alerting above 100% is valid.
const CAPPED_PERCENT_METRICS: ReadonlySet<AlertMetric> = new Set<AlertMetric>([
  'cpu_usage_percent',
  'memory_usage_percent',
  'disk_usage_percent',
]);

export function maxThresholdFor(metric: AlertMetric): number | undefined {
  return CAPPED_PERCENT_METRICS.has(metric) ? 100 : undefined;
}
