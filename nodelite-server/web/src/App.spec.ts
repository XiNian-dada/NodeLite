import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { createApp, defineComponent, h } from 'vue';

import App from './App.vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';

const FAKE_DICT = {
  en: { 'common.theme_toggle': 'Toggle theme', 'common.language': 'Language' },
  'zh-CN': { 'common.theme_toggle': '切换主题', 'common.language': '语言' },
};

/* eslint-disable vue/one-component-per-file -- test scaffolding uses multiple tiny placeholder components */
const Placeholder = defineComponent({ render: () => h('div', 'placeholder') });
const Dummy = defineComponent({ render: () => null });
/* eslint-enable vue/one-component-per-file */

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/', name: 'dashboard', component: Placeholder },
    { path: '/nodes/:id', name: 'node-detail', component: Placeholder },
  ],
});

let pinia: ReturnType<typeof createPinia>;

describe('App.vue', () => {
  beforeEach(async () => {
    window.localStorage.clear();
    __resetI18nForTest();
    pinia = createPinia();
    setActivePinia(pinia);
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response),
    );
    // Initialize the vue-i18n singleton on a throwaway app so the App.vue
    // mount below sees a wired-up instance via getI18n().
    const dummy = createApp(Dummy);
    await setupI18n(dummy);
  });

  afterEach(() => {
    window.localStorage.clear();
    __resetI18nForTest();
    vi.unstubAllGlobals();
  });

  it('renders the app shell with theme + language controls', async () => {
    await router.push('/');
    await router.isReady();

    const wrapper = mount(App, {
      global: {
        plugins: [pinia, router, getI18n()],
      },
    });
    await flushPromises();

    expect(wrapper.find('[data-test="app-shell"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="theme-toggle"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="language-select"]').exists()).toBe(true);
  });

  it('toggle theme flips html data-theme attribute', async () => {
    document.documentElement.dataset.theme = 'dark';
    await router.push('/');
    await router.isReady();

    const wrapper = mount(App, {
      global: {
        plugins: [pinia, router, getI18n()],
      },
    });
    await flushPromises();

    await wrapper.find('[data-test="theme-toggle"]').trigger('click');
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
