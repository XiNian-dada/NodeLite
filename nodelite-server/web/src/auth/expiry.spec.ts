import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AUTH_EXPIRY_MS,
  AUTH_TIMESTAMP_KEY,
  LOGOUT_PATH,
  checkAuthExpiry,
} from './expiry';

describe('checkAuthExpiry', () => {
  let assignSpy: ReturnType<typeof vi.fn>;
  let originalLocation: Location;

  beforeEach(() => {
    window.localStorage.clear();
    originalLocation = window.location;
    assignSpy = vi.fn();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: { assign: assignSpy, href: originalLocation.href },
    });
  });

  afterEach(() => {
    window.localStorage.clear();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: originalLocation,
    });
    vi.useRealTimers();
  });

  it('writes current timestamp and continues when no timestamp exists', () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_700_000_000_000);

    expect(checkAuthExpiry()).toBe(true);
    expect(window.localStorage.getItem(AUTH_TIMESTAMP_KEY)).toBe('1700000000000');
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it('refreshes the timestamp when within the 24h window', () => {
    vi.useFakeTimers();
    const past = 1_700_000_000_000;
    const now = past + 60_000;
    window.localStorage.setItem(AUTH_TIMESTAMP_KEY, past.toString());
    vi.setSystemTime(now);

    expect(checkAuthExpiry()).toBe(true);
    expect(window.localStorage.getItem(AUTH_TIMESTAMP_KEY)).toBe(now.toString());
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it('clears the timestamp and redirects when older than 24h', () => {
    vi.useFakeTimers();
    const past = 1_700_000_000_000;
    const now = past + AUTH_EXPIRY_MS + 1;
    window.localStorage.setItem(AUTH_TIMESTAMP_KEY, past.toString());
    vi.setSystemTime(now);

    expect(checkAuthExpiry()).toBe(false);
    expect(window.localStorage.getItem(AUTH_TIMESTAMP_KEY)).toBeNull();
    expect(assignSpy).toHaveBeenCalledWith(LOGOUT_PATH);
  });

  it('treats a non-numeric stored value as missing (re-seeds)', () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_700_000_000_000);
    window.localStorage.setItem(AUTH_TIMESTAMP_KEY, 'not-a-number');

    expect(checkAuthExpiry()).toBe(true);
    expect(window.localStorage.getItem(AUTH_TIMESTAMP_KEY)).toBe('1700000000000');
    expect(assignSpy).not.toHaveBeenCalled();
  });
});
