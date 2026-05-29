import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope } from 'vue';
import { usePolling } from './usePolling';

describe('usePolling', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    Object.defineProperty(document, 'hidden', {
      configurable: true,
      value: false,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('calls fn once immediately on setup', () => {
    const scope = effectScope();
    const fn = vi.fn();
    scope.run(() => {
      usePolling(fn, 1000);
    });
    expect(fn).toHaveBeenCalledTimes(1);
    scope.stop();
  });

  it('calls fn every intervalMs', () => {
    const scope = effectScope();
    const fn = vi.fn();
    scope.run(() => {
      usePolling(fn, 1000);
    });

    expect(fn).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(3000);
    expect(fn).toHaveBeenCalledTimes(5);

    scope.stop();
  });

  it('stops calling fn after scope is disposed', () => {
    const scope = effectScope();
    const fn = vi.fn();
    scope.run(() => {
      usePolling(fn, 1000);
    });

    expect(fn).toHaveBeenCalledTimes(1);
    scope.stop();
    vi.advanceTimersByTime(5000);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('skips ticks when document.hidden is true', () => {
    Object.defineProperty(document, 'hidden', {
      configurable: true,
      value: true,
    });
    const scope = effectScope();
    const fn = vi.fn();
    scope.run(() => {
      usePolling(fn, 1000);
    });

    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(5000);
    expect(fn).not.toHaveBeenCalled();

    scope.stop();
  });
});
