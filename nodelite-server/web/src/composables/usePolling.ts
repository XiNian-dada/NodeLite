import { getCurrentScope, onScopeDispose } from 'vue';

/**
 * Call `fn()` immediately, then every `intervalMs` ms. Skips a tick when
 * the page is hidden (matches legacy assets/index.html:2429 behavior).
 * Cleans up via the current Vue effect scope — so when the owning
 * component unmounts, the interval clears automatically.
 *
 * The stores in src/stores/* are pure state — the polling lifecycle
 * lives here instead of on the store, because Pinia stores are
 * singletons and per-component start/stop would race across routes.
 */
export function usePolling(fn: () => void | Promise<void>, intervalMs: number): void {
  const tick = (): void => {
    if (typeof document !== 'undefined' && document.hidden) return;
    void fn();
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
