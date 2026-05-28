import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';

import App from './App.vue';

const sampleBootstrap = {
  service: 'nodelite-server',
  status: 'ready',
  ready: true,
  history_available: true,
  public_base_url: null,
  refresh_interval_secs: 5,
  registered_nodes: 3,
};

describe('App.vue (hello world)', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('renders bootstrap data on success', async () => {
    (globalThis.fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      ok: true,
      json: async () => sampleBootstrap,
    });

    const wrapper = mount(App);
    await flushPromises();

    const ok = wrapper.find('[data-test="bootstrap-ok"]');
    expect(ok.exists()).toBe(true);
    expect(ok.text()).toContain('nodelite-server');
    expect(ok.text()).toContain('ready');
  });

  it('shows error state on http failure', async () => {
    (globalThis.fetch as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      ok: false,
      status: 503,
      json: async () => ({}),
    });

    const wrapper = mount(App);
    await flushPromises();

    const err = wrapper.find('[data-test="bootstrap-error"]');
    expect(err.exists()).toBe(true);
    expect(err.text()).toContain('HTTP 503');
  });
});
