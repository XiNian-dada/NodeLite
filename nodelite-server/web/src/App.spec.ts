import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { defineComponent, h } from 'vue';
import { createI18n } from 'vue-i18n';
import { createMemoryHistory, createRouter } from 'vue-router';
import App from './App.vue';
import NodeList from '@/components/NodeList.vue';
import { makeNode, makeOverview } from '@/api/__fixtures__/nodes';
import type { BrowserMessage } from '@/api/types';
import { useNodesStore } from '@/stores/nodes';
import { useOverviewStore } from '@/stores/overview';

const socket = vi.hoisted(() => {
  const handlers = new Map<string, Set<(message: never) => void>>();
  return {
    handlers,
    connect: vi.fn(),
    destroy: vi.fn(),
    on: vi.fn((type: string, handler: (message: never) => void) => {
      const set = handlers.get(type) ?? new Set();
      set.add(handler);
      handlers.set(type, set);
      return () => set.delete(handler);
    }),
  };
});
vi.mock('@/ws', () => ({ useWebSocket: () => socket }));

function emit(message: BrowserMessage): void {
  for (const handler of socket.handlers.get(message.type) ?? []) handler(message as never);
}

const placeholder = defineComponent({ render: () => h('div', { 'data-test': 'route-stub' }) });
const wrappers = new Set<{ unmount(): void }>();

async function mountApp() {
  const pinia = createPinia();
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: NodeList },
      ...['settings', 'account', 'alerts', 'logs'].map((path) => ({
        path: `/${path}`,
        component: placeholder,
      })),
      { path: '/nodes/:id', component: placeholder },
    ],
  });
  await router.push('/');
  const wrapper = mount(App, {
    global: {
      plugins: [
        pinia,
        router,
        createI18n({ legacy: false, locale: 'en', missingWarn: false, fallbackWarn: false }),
      ],
    },
  });
  wrappers.add(wrapper);
  await flushPromises();
  return { wrapper, router, nodes: useNodesStore(pinia), overview: useOverviewStore(pinia) };
}

describe('App realtime data ownership', () => {
  beforeEach(() => {
    socket.handlers.clear();
    vi.clearAllMocks();
  });
  afterEach(() => {
    for (const wrapper of wrappers) wrapper.unmount();
    wrappers.clear();
  });

  it.each(['settings', 'account', 'alerts', 'logs'])(
    'consumes updates and removals while on /%s using one connection',
    async (path) => {
      const { wrapper, router, overview } = await mountApp();
      const first = makeNode();
      const second = makeNode({
        identity: { ...first.identity, node_id: 'node-b', node_label: 'Node B' },
      });
      emit({
        type: 'initial_state',
        generated_at: '2026-06-01T12:00:00Z',
        nodes: [first, second],
        overview: makeOverview(),
      });
      await flushPromises();
      expect(wrapper.findAll('[data-test="node-card"]')).toHaveLength(2);
      await router.push(`/${path}`);
      expect(wrapper.find('[data-test="route-stub"]').exists()).toBe(true);
      emit({
        type: 'node_upsert',
        generated_at: '2026-06-01T12:00:01Z',
        node: { ...first, identity: { ...first.identity, node_label: 'Updated while away' } },
      });
      emit({
        type: 'node_removed',
        generated_at: '2026-06-01T12:00:02Z',
        node_id: second.identity.node_id,
      });
      emit({
        type: 'overview_update',
        generated_at: '2026-06-01T12:00:03Z',
        overview: makeOverview({ total_nodes: 1 }),
      });
      await router.push('/');
      await flushPromises();
      expect(wrapper.findAll('[data-test="node-card"]')).toHaveLength(1);
      expect(wrapper.text()).toContain('Updated while away');
      expect(wrapper.text()).not.toContain('Node B');
      expect(overview.data?.total_nodes).toBe(1);
      expect(socket.connect).toHaveBeenCalledTimes(1);
      expect(socket.on).toHaveBeenCalledTimes(4);
    },
  );

  it('removes consumers on unmount and subscribes once on remount', async () => {
    const first = await mountApp();
    first.wrapper.unmount();
    wrappers.delete(first.wrapper);
    expect(socket.destroy).toHaveBeenCalledTimes(1);
    expect([...socket.handlers.values()].every((set) => set.size === 0)).toBe(true);
    const second = await mountApp();
    const upsert = vi.spyOn(second.nodes, 'upsertNode');
    emit({ type: 'node_upsert', generated_at: '2026-06-01T12:00:00Z', node: makeNode() });
    expect(upsert).toHaveBeenCalledTimes(1);
    expect(first.nodes.nodes).toHaveLength(0);
  });
});
