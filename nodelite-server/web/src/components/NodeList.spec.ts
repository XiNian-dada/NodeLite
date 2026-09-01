import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises, RouterLinkStub } from '@vue/test-utils';
import { createApp, defineComponent, h } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { NODE_CARD_HEIGHT, NODE_GRID_GAP } from '@/lib/gridVirtualizer';
import { useNodesStore } from '@/stores/nodes';
import { makeNode } from '@/api/__fixtures__/nodes';
import NodeList from './NodeList.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: { ...actual.apiClient, nodeHistory: vi.fn().mockResolvedValue([]) },
  };
});

const FAKE_DICT = {
  en: {
    'common.waiting_for_data': 'Waiting for data…',
    'index.node.latency': 'Latency',
    'index.node.load': 'Load',
    'index.node.cpu': 'CPU',
    'index.node.memory': 'Memory',
    'index.node.memory_used': 'Memory',
    'index.node.service_expiry': 'Service expiry',
    'index.node.renewal_price': 'Renewal price',
    'index.node.service_unlimited': 'Unlimited',
    'index.node.self_owned': 'Self-owned',
    'common.online': 'Online',
    'common.offline': 'Offline',
    'common.latency_warn': 'High latency',
  },
  'zh-CN': {
    'common.waiting_for_data': '等待数据…',
    'index.node.memory': '内存',
    'index.node.memory_used': '内存',
    'index.node.service_expiry': '服务到期',
    'index.node.renewal_price': '续费价格',
    'index.node.service_unlimited': '无限制',
    'index.node.self_owned': '自持有',
  },
};

const Stub = defineComponent({ render: () => h('div') });
const mountedWrappers = new Set<{ unmount: () => void }>();
const originalInnerWidth = Object.getOwnPropertyDescriptor(window, 'innerWidth')!;
const originalInnerHeight = Object.getOwnPropertyDescriptor(window, 'innerHeight')!;
const originalScrollY = Object.getOwnPropertyDescriptor(window, 'scrollY')!;
let scrollY = 0;
let animationFrames: FrameRequestCallback[] = [];

function runAnimationFrames(): void {
  const pending = animationFrames;
  animationFrames = [];
  pending.forEach((callback) => callback(0));
}

async function mountList(nodes: ReturnType<typeof makeNode>[]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useNodesStore();
  store.applyServerState(nodes, new Date().toISOString());
  const wrapper = mount(NodeList, {
    global: {
      plugins: [pinia, getI18n()],
      stubs: { RouterLink: RouterLinkStub },
    },
  });
  mountedWrappers.add(wrapper);
  await flushPromises();
  return wrapper;
}

describe('NodeList', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    scrollY = 0;
    animationFrames = [];
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1280 });
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 800 });
    Object.defineProperty(window, 'scrollY', { configurable: true, get: () => scrollY });
    vi.stubGlobal(
      'requestAnimationFrame',
      vi.fn((callback: FrameRequestCallback) => {
        animationFrames.push(callback);
        return animationFrames.length;
      }),
    );
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response),
    );
    const dummy = createApp(Stub);
    await setupI18n(dummy);
  });

  afterEach(() => {
    for (const wrapper of mountedWrappers) wrapper.unmount();
    mountedWrappers.clear();
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
    Object.defineProperty(window, 'innerWidth', originalInnerWidth);
    Object.defineProperty(window, 'innerHeight', originalInnerHeight);
    Object.defineProperty(window, 'scrollY', originalScrollY);
  });

  it('renders one card per node, keyed by node id', async () => {
    const wrapper = await mountList([
      makeNode({ identity: { node_id: 'a', node_label: 'A', hostname: 'ha', tags: [] } }),
      makeNode({ identity: { node_id: 'b', node_label: 'B', hostname: 'hb', tags: [] } }),
      makeNode({ identity: { node_id: 'c', node_label: 'C', hostname: 'hc', tags: [] } }),
    ]);
    expect(wrapper.findAll('[data-test="node-card"]')).toHaveLength(3);
    expect(wrapper.find('[data-test="node-list-empty"]').exists()).toBe(false);
  });

  it('shows the empty state when there are no nodes', async () => {
    const wrapper = await mountList([]);
    expect(wrapper.findAll('[data-test="node-card"]')).toHaveLength(0);
    expect(wrapper.find('[data-test="node-list-empty"]').text()).toBe('Waiting for data…');
  });

  it('keeps the mounted card count below 50 for 500 nodes', async () => {
    const nodes = Array.from({ length: 500 }, (_, index) =>
      makeNode({
        identity: {
          node_id: `node-${String(index).padStart(3, '0')}`,
          node_label: `Node ${index}`,
          hostname: `host-${index}`,
          tags: [],
        },
      }),
    );
    const wrapper = await mountList(nodes);

    const cards = wrapper.findAll('[data-test="node-card"]');
    expect(cards.length).toBeGreaterThan(0);
    expect(cards.length).toBeLessThan(50);
    expect(wrapper.find('[data-test="node-grid-window"]').attributes('data-start-index')).toBe('0');
  });

  it('moves the rendered node window after scrolling', async () => {
    const nodes = Array.from({ length: 500 }, (_, index) =>
      makeNode({
        identity: {
          node_id: `node-${String(index).padStart(3, '0')}`,
          node_label: `Node ${index}`,
          hostname: `host-${index}`,
          tags: [],
        },
      }),
    );
    const wrapper = await mountList(nodes);
    const firstNodeId = wrapper.find('[data-test="node-card"]').attributes('data-node-id');

    scrollY = (NODE_CARD_HEIGHT + NODE_GRID_GAP) * 20;
    window.dispatchEvent(new Event('scroll'));
    runAnimationFrames();
    await flushPromises();

    const cards = wrapper.findAll('[data-test="node-card"]');
    expect(cards.length).toBeLessThan(50);
    expect(cards[0]?.attributes('data-node-id')).not.toBe(firstNodeId);
    expect(wrapper.find('[data-test="node-grid-window"]').attributes('data-start-index')).toBe(
      '54',
    );
  });

  it('keeps visible cards linked to their detail route', async () => {
    const wrapper = await mountList([
      makeNode({
        identity: { node_id: 'srv 1', node_label: 'Server', hostname: 'host', tags: [] },
      }),
    ]);

    expect(wrapper.findComponent(RouterLinkStub).props('to')).toBe('/nodes/srv%201');
  });
});
