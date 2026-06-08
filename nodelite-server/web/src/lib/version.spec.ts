import { describe, expect, it } from 'vitest';
import { compareVersions, isNewerVersion, isStableVersionTag, normalizeVersionTag } from './version';

describe('compareVersions', () => {
  it('orders by numeric segments', () => {
    expect(compareVersions('1.2.3', '1.2.3')).toBe(0);
    expect(compareVersions('1.2.4', '1.2.3')).toBeGreaterThan(0);
    expect(compareVersions('1.2.3', '1.3.0')).toBeLessThan(0);
    expect(compareVersions('2.0.0', '1.9.9')).toBeGreaterThan(0);
  });

  it('treats missing segments as 0', () => {
    expect(compareVersions('1.2', '1.2.0')).toBe(0);
    expect(compareVersions('1.2.1', '1.2')).toBeGreaterThan(0);
  });

  it('treats non-numeric segments as 0', () => {
    expect(compareVersions('1.x.0', '1.0.0')).toBe(0);
  });
});

describe('normalizeVersionTag', () => {
  it('strips a leading v', () => {
    expect(normalizeVersionTag('v2.3.0')).toBe('2.3.0');
    expect(normalizeVersionTag(' V2.3.0 ')).toBe('2.3.0');
    expect(normalizeVersionTag('2.3.0')).toBe('2.3.0');
  });
});

describe('isStableVersionTag', () => {
  it('accepts stable release tags', () => {
    expect(isStableVersionTag('v2.3.0')).toBe(true);
    expect(isStableVersionTag('2.3')).toBe(true);
    expect(isStableVersionTag('2.3.0+build.1')).toBe(true);
  });

  it('rejects prerelease and test tags', () => {
    expect(isStableVersionTag('v2.4.0-rc.1')).toBe(false);
    expect(isStableVersionTag('v2.4.0-beta.1')).toBe(false);
    expect(isStableVersionTag('v2.4.0-test')).toBe(false);
    expect(isStableVersionTag('latest')).toBe(false);
  });
});

describe('isNewerVersion', () => {
  it('detects a newer release tag vs the current version', () => {
    expect(isNewerVersion('v2.3.1', '2.3.0')).toBe(true);
    expect(isNewerVersion('v2.3.0', '2.3.0')).toBe(false);
    expect(isNewerVersion('v2.2.9', '2.3.0')).toBe(false);
  });
});
