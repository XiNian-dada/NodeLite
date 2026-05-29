import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope } from 'vue';
import { ApiAbortError } from '@/api/client';
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

  it('swallows ApiAbortError without logging or crashing the interval', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const scope = effectScope();
    const fn = vi.fn().mockRejectedValue(new ApiAbortError('redirecting'));
    scope.run(() => {
      usePolling(fn, 1000);
    });

    expect(fn).toHaveBeenCalledTimes(1);
    // Let the rejected promise settle on the microtask queue.
    await Promise.resolve();
    expect(errorSpy).not.toHaveBeenCalled();

    // The interval keeps firing despite the rejection.
    vi.advanceTimersByTime(2000);
    expect(fn).toHaveBeenCalledTimes(3);

    scope.stop();
    // Flush the later ticks' rejections while the spy is still mocked.
    await Promise.resolve();
    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('logs a generic error but keeps the interval alive', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const scope = effectScope();
    const fn = vi.fn().mockRejectedValue(new Error('boom'));
    scope.run(() => {
      usePolling(fn, 1000);
    });

    expect(fn).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    expect(errorSpy).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(2000);
    expect(fn).toHaveBeenCalledTimes(3);

    scope.stop();
    // Flush the later ticks' rejections while the spy is still mocked.
    await Promise.resolve();
    expect(errorSpy).toHaveBeenCalledTimes(3);
    errorSpy.mockRestore();
  });
});
