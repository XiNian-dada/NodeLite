import { describe, expect, it } from 'vitest';
import {
  fmtBytes,
  fmtLatency,
  fmtPercent,
  fmtRate,
  tokenRemaining,
  tokenSeverity,
  uptimeParts,
} from './format';

describe('fmtBytes', () => {
  it('scales by 1024 with legacy decimal rules', () => {
    expect(fmtBytes(0)).toBe('0 B');
    expect(fmtBytes(512)).toBe('512 B');
    expect(fmtBytes(1024)).toBe('1.0 KB');
    expect(fmtBytes(1536)).toBe('1.5 KB');
    expect(fmtBytes(150 * 1024)).toBe('150 KB'); // >=100 → 0 decimals
    expect(fmtBytes(8_000_000_000)).toBe('7.5 GB');
  });

  it('returns null for null/NaN', () => {
    expect(fmtBytes(null)).toBeNull();
    expect(fmtBytes(undefined)).toBeNull();
    expect(fmtBytes(Number.NaN)).toBeNull();
  });
});

describe('fmtRate', () => {
  it('converts bytes/sec to bits/sec scaled by 1000', () => {
    expect(fmtRate(0)).toBe('0 bps');
    expect(fmtRate(10)).toBe('80 bps'); // 10 * 8 = 80 bps (i===0 → 0 decimals)
    expect(fmtRate(125)).toBe('1.0 Kbps'); // 1000 bps → 1.0 Kbps
    expect(fmtRate(125_000)).toBe('1.0 Mbps'); // 1,000,000 bps
  });

  it('returns null for null', () => {
    expect(fmtRate(null)).toBeNull();
  });
});

describe('fmtPercent', () => {
  it('formats one decimal', () => {
    expect(fmtPercent(63.74)).toBe('63.7%');
    expect(fmtPercent(0)).toBe('0.0%');
  });
  it('returns null for null/NaN', () => {
    expect(fmtPercent(null)).toBeNull();
    expect(fmtPercent(Number.NaN)).toBeNull();
  });
});

describe('fmtLatency', () => {
  it('rounds to whole ms', () => {
    expect(fmtLatency(8.6)).toBe('9 ms');
    expect(fmtLatency(0)).toBe('0 ms');
  });
  it('returns null for null', () => {
    expect(fmtLatency(null)).toBeNull();
  });
});

describe('uptimeParts', () => {
  it('breaks seconds into days/hours/minutes', () => {
    expect(uptimeParts(0)).toEqual({ days: 0, hours: 0, minutes: 0 });
    expect(uptimeParts(90 * 60)).toEqual({ days: 0, hours: 1, minutes: 30 });
    expect(uptimeParts((25 * 60 + 5) * 60)).toEqual({ days: 1, hours: 1, minutes: 5 });
  });
  it('returns null for null/NaN', () => {
    expect(uptimeParts(null)).toBeNull();
    expect(uptimeParts(Number.NaN)).toBeNull();
  });
});

describe('tokenRemaining', () => {
  it('maps null/NaN to none, <=0 to expired', () => {
    expect(tokenRemaining(null)).toEqual({ kind: 'none' });
    expect(tokenRemaining(Number.NaN)).toEqual({ kind: 'none' });
    expect(tokenRemaining(0)).toEqual({ kind: 'expired' });
    expect(tokenRemaining(-5)).toEqual({ kind: 'expired' });
  });

  it('uses days_hours when >=1 day, minutes (min 1) otherwise', () => {
    expect(tokenRemaining((25 * 3600 + 5))).toEqual({ kind: 'days_hours', days: 1, hours: 1 });
    expect(tokenRemaining(90 * 60)).toEqual({ kind: 'minutes', minutes: 90 });
    expect(tokenRemaining(10)).toEqual({ kind: 'minutes', minutes: 1 }); // floor → 0, clamped to 1
  });
});

describe('tokenSeverity', () => {
  it('classifies by remaining seconds', () => {
    expect(tokenSeverity(null)).toBe('');
    expect(tokenSeverity(0)).toBe('expired');
    expect(tokenSeverity(3 * 86400)).toBe('expiring'); // < 7 days
    expect(tokenSeverity(30 * 86400)).toBe('ok');
  });
});
