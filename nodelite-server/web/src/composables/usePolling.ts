import { getCurrentScope, onScopeDispose } from 'vue';
import { ApiAbortError } from '@/api/client';

/**
 * Call `fn()` immediately, then every `intervalMs` ms. Skips a tick when
 * the page is hidden (matches legacy assets/index.html:2429 behavior).
 * Cleans up via the current Vue effect scope — so when the owning
 * component unmounts, the interval clears automatically.
 *
 * The stores in src/stores/* are pure state — the polling lifecycle
 * lives here instead of on the store, because Pinia stores are
 * singletons and per-component start/stop would race across routes.
 *
 * An ApiAbortError means the API client has already initiated a full-page
 * navigation (verify-2fa / logout) — swallow it silently. Any other error
 * is logged but never crashes the interval, so a single failed tick
 * doesn't stop subsequent polls.
 */
export function usePolling(fn: () => void | Promise<void>, intervalMs: number): void {
  const onError = (e: unknown): void => {
    if (!(e instanceof ApiAbortError)) {
      console.error('polling tick failed', e);
    }
  };

  const tick = (): void => {
    if (typeof document !== 'undefined' && document.hidden) return;
    try {
      const result = fn();
      if (result instanceof Promise) {
        result.catch(onError);
      }
    } catch (e) {
      onError(e);
    }
  };

  tick();
  const handle = window.setInterval(tick, intervalMs);

  const cleanup = (): void => {
    window.clearInterval(handle);
  };

  if (getCurrentScope() !== undefined) {
    onScopeDispose(cleanup);
  }
}
