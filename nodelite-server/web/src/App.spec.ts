import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';

import App from './App.vue';

const Placeholder = defineComponent({ render: () => h('div', { 'data-test': 'route-stub' }) });

const router = createRouter({
  history: createMemoryHistory(),
  routes: [{ path: '/', name: 'dashboard', component: Placeholder }],
});

describe('App.vue', () => {
  it('renders the active route via RouterView', async () => {
    await router.push('/');
    await router.isReady();

    const wrapper = mount(App, { global: { plugins: [router] } });
    expect(wrapper.find('[data-test="route-stub"]').exists()).toBe(true);
  });
});
