import { ref, type Ref } from 'vue';

export interface DedupeAsyncContext {
  isCurrent: () => boolean;
  signal: AbortSignal;
}

interface PendingRequest<K> {
  key: K;
  promise: Promise<unknown>;
  controller: AbortController;
  requestId: number;
}

export interface DedupeAsync<K> {
  inFlightKey: Ref<K | null>;
  isCurrent: (key: K) => boolean;
  run: <T>(key: K, task: (context: DedupeAsyncContext) => Promise<T>) => Promise<T>;
  abort: () => void;
}

export function useDedupeAsync<K>(): DedupeAsync<K> {
  const inFlightKey = ref<K | null>(null) as Ref<K | null>;
  let pending: PendingRequest<K> | null = null;
  let nextRequestId = 0;

  function isCurrent(key: K): boolean {
    return pending?.key === key && inFlightKey.value === key;
  }

  function run<T>(key: K, task: (context: DedupeAsyncContext) => Promise<T>): Promise<T> {
    if (pending?.key === key) return pending.promise as Promise<T>;

    const controller = new AbortController();
    const requestId = nextRequestId + 1;
    nextRequestId = requestId;
    inFlightKey.value = key;

    const request: PendingRequest<K> = {
      key,
      promise: Promise.resolve(undefined),
      controller,
      requestId,
    };
    pending = request;

    let taskPromise: Promise<T>;
    try {
      taskPromise = task({
        isCurrent: () => pending?.requestId === requestId,
        signal: controller.signal,
      });
    } catch (e) {
      taskPromise = Promise.reject(e);
    }

    const promise = taskPromise.finally(() => {
      if (pending?.requestId === requestId && pending.promise === promise) {
        pending = null;
        inFlightKey.value = null;
      }
    });

    request.promise = promise;
    return promise;
  }

  function abort(): void {
    if (pending === null) return;
    pending.controller.abort();
    pending = null;
    inFlightKey.value = null;
  }

  return { inFlightKey, isCurrent, run, abort };
}
