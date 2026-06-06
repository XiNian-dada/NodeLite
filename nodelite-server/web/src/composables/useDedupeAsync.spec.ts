import { describe, expect, it, vi } from 'vitest';
import { useDedupeAsync } from './useDedupeAsync';

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
} {
  let resolve: (value: T) => void = () => {};
  let reject: (reason: unknown) => void = () => {};
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

describe('useDedupeAsync', () => {
  it('merges concurrent requests with the same key', async () => {
    const runner = useDedupeAsync<string>();
    const request = deferred<string>();
    const task = vi.fn(() => request.promise);

    const first = runner.run('node-a', task);
    const second = runner.run('node-a', task);

    expect(first).toBe(second);
    expect(task).toHaveBeenCalledTimes(1);
    expect(runner.inFlightKey.value).toBe('node-a');
    expect(runner.isCurrent('node-a')).toBe(true);

    request.resolve('ok');
    await expect(first).resolves.toBe('ok');
    await expect(second).resolves.toBe('ok');
    expect(runner.inFlightKey.value).toBeNull();
  });

  it('lets a newer key take over without letting the old request clear it', async () => {
    const runner = useDedupeAsync<string>();
    const oldRequest = deferred<string>();
    const newRequest = deferred<string>();
    let oldIsCurrent = (): boolean => false;
    let newIsCurrent = (): boolean => false;

    const oldPromise = runner.run('node-a', ({ isCurrent }) => {
      oldIsCurrent = isCurrent;
      return oldRequest.promise;
    });
    const newPromise = runner.run('node-b', ({ isCurrent }) => {
      newIsCurrent = isCurrent;
      return newRequest.promise;
    });

    expect(runner.inFlightKey.value).toBe('node-b');
    expect(runner.isCurrent('node-a')).toBe(false);
    expect(runner.isCurrent('node-b')).toBe(true);
    expect(oldIsCurrent()).toBe(false);
    expect(newIsCurrent()).toBe(true);

    oldRequest.resolve('old');
    await expect(oldPromise).resolves.toBe('old');
    expect(runner.inFlightKey.value).toBe('node-b');

    newRequest.resolve('new');
    await expect(newPromise).resolves.toBe('new');
    expect(runner.inFlightKey.value).toBeNull();
  });

  it('does not treat an old request as current after the same key starts again', async () => {
    const runner = useDedupeAsync<string>();
    const firstRequest = deferred<string>();
    const middleRequest = deferred<string>();
    const latestRequest = deferred<string>();
    let firstIsCurrent = (): boolean => false;
    let latestIsCurrent = (): boolean => false;

    const firstPromise = runner.run('node-a', ({ isCurrent }) => {
      firstIsCurrent = isCurrent;
      return firstRequest.promise;
    });
    const middlePromise = runner.run('node-b', () => middleRequest.promise);
    const latestPromise = runner.run('node-a', ({ isCurrent }) => {
      latestIsCurrent = isCurrent;
      return latestRequest.promise;
    });

    expect(runner.inFlightKey.value).toBe('node-a');
    expect(firstIsCurrent()).toBe(false);
    expect(latestIsCurrent()).toBe(true);

    firstRequest.resolve('first');
    await expect(firstPromise).resolves.toBe('first');
    expect(runner.inFlightKey.value).toBe('node-a');
    expect(firstIsCurrent()).toBe(false);
    expect(latestIsCurrent()).toBe(true);

    middleRequest.resolve('middle');
    await expect(middlePromise).resolves.toBe('middle');
    expect(runner.inFlightKey.value).toBe('node-a');

    latestRequest.resolve('latest');
    await expect(latestPromise).resolves.toBe('latest');
    expect(runner.inFlightKey.value).toBeNull();
  });

  it('aborts the current controller and clears the active key', async () => {
    const runner = useDedupeAsync<string>();
    const request = deferred<void>();
    const signals: AbortSignal[] = [];

    const promise = runner.run('node-a', ({ signal }) => {
      signals.push(signal);
      return request.promise;
    });

    expect(signals[0]?.aborted).toBe(false);
    runner.abort();
    expect(signals[0]?.aborted).toBe(true);
    expect(runner.inFlightKey.value).toBeNull();

    request.resolve();
    await promise;
  });

  it('clears the active key when the task rejects', async () => {
    const runner = useDedupeAsync<string>();
    const task = vi.fn(() => Promise.reject(new Error('boom')));

    await expect(runner.run('node-a', task)).rejects.toThrow('boom');

    expect(task).toHaveBeenCalledTimes(1);
    expect(runner.inFlightKey.value).toBeNull();
  });
});
