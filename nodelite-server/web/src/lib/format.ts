/**
 * Pure value formatters, ported from the legacy fmt* helpers in
 * assets/node.html:1567-1612. i18n-free: null/NaN inputs return null so the
 * caller can render `value ?? $t('common.not_available')`. Uptime returns a
 * structured breakdown the caller maps to the node.uptime.* i18n keys.
 */

function scaleUnit(value: number, units: readonly string[], step: number): string {
  let v = value;
  let i = 0;
  while (v >= step && i < units.length - 1) {
    v /= step;
    i += 1;
  }
  const decimals = v >= 100 || i === 0 ? 0 : 1;
  return `${v.toFixed(decimals)} ${units[i]}`;
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const;
const RATE_UNITS = ['bps', 'Kbps', 'Mbps', 'Gbps', 'Tbps'] as const;

export function fmtBytes(bytes: number | null | undefined): string | null {
  if (bytes == null || !Number.isFinite(Number(bytes))) return null;
  return scaleUnit(Number(bytes), BYTE_UNITS, 1024);
}

/** Bytes/sec → bits/sec rate (×8), scaled by 1000 like the legacy fmtRate. */
export function fmtRate(bytesPerSec: number | null | undefined): string | null {
  if (bytesPerSec == null || !Number.isFinite(Number(bytesPerSec))) return null;
  return scaleUnit(Number(bytesPerSec) * 8, RATE_UNITS, 1000);
}

export function fmtPercent(value: number | null | undefined): string | null {
  if (value == null || !Number.isFinite(Number(value))) return null;
  return `${Number(value).toFixed(1)}%`;
}

export function fmtLatency(value: number | null | undefined): string | null {
  if (value == null || !Number.isFinite(Number(value))) return null;
  return `${Math.round(Number(value))} ms`;
}

export interface UptimeParts {
  days: number;
  hours: number;
  minutes: number;
}

/**
 * Break uptime seconds into days/hours/minutes. Returns null for null/NaN.
 * The caller picks the i18n key: days>0 → node.uptime.days_hours,
 * hours>0 → node.uptime.hours_minutes, else node.uptime.minutes.
 */
export function uptimeParts(seconds: number | null | undefined): UptimeParts | null {
  if (seconds == null || !Number.isFinite(Number(seconds))) return null;
  const totalMinutes = Math.floor(Number(seconds) / 60);
  const days = Math.floor(totalMinutes / (24 * 60));
  const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
  const minutes = totalMinutes % 60;
  return { days, hours, minutes };
}
