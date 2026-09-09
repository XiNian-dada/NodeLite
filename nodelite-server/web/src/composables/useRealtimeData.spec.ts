import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { defineComponent, h } from 'vue';
import { createI18n } from 'vue-i18n';
import type { ConnectionState } from '@/ws/client';
import type { BrowserMessage } from '@/api/types';
import { apiClient } from '@/api';
import { makeNode, makeOverview } from '@/api/__fixtures__/nodes';
import { useNodesStore } from '@/stores/nodes';
import { useOverviewStore } from '@/stores/overview';
import { useRealtimeStore } from '@/stores/realtime';
import ConnectionStatus from '@/components/ConnectionStatus.vue';
import { useRealtimeData } from './useRealtimeData';

const socket = vi.hoisted(() => {
  const messages = new Map<string, (message: never) => void>();
  const states = new Set<(state: ConnectionState) => void>();
  return {
    messages,
    states,
    connect: vi.fn(),
    destroy: vi.fn(),
    on: (type: string, handler: (message: never) => void) => {
      messages.set(type, handler);
      return () => messages.delete(type);
    },
    onState: (handler: (state: ConnectionState) => void) => {
      states.add(handler);
      handler({ kind: 'idle' });
      return () => states.delete(handler);
    },
    transition: (state: ConnectionState) => {
      for (const handler of states) handler(state);
    },
  };
});
vi.mock('@/ws', () => ({ useWebSocket: () => socket }));
vi.mock('@/api', () => ({ apiClient: { listNodes: vi.fn(), overview: vi.fn() } }));

function emit(message: BrowserMessage): void {
  socket.messages.get(message.type)?.(message as never);
}

function initial(label: string): void {
  const node = makeNode();
  emit({
    type: 'initial_state',
    generated_at: new Date().toISOString(),
    overview: makeOverview(),
    nodes: [{ ...node, identity: { ...node.identity, node_label: label } }],
  });
}

describe('continuous REST fallback', () => {
  let wrapper: VueWrapper;
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-06-01T12:00:00Z'));
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    vi.mocked(apiClient.listNodes).mockReset().mockResolvedValue([makeNode()]);
    vi.mocked(apiClient.overview).mockReset().mockResolvedValue(makeOverview());
    const Root = defineComponent({
      setup() {
        useRealtimeData();
        return () => h(ConnectionStatus);
      },
    });
    wrapper = mount(Root, {
      global: { plugins: [createPinia(), createI18n({ legacy: false, locale: 'en' })] },
    });
  });
  afterEach(() => {
    wrapper.unmount();
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it('survives three failed handshakes, shows stale data, keeps polling, and stops after WS recovery', async () => {
    initial('Live A');
    await flushPromises();
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    vi.mocked(apiClient.listNodes)
      .mockRejectedValueOnce(new Error('offline'))
      .mockRejectedValueOnce(new Error('offline'));
    socket.transition({ kind: 'reconnecting', attempt: 1, nextAttemptAt: Date.now() + 2000 });
    await vi.advanceTimersByTimeAsync(500);
    for (const attempt of [2, 3, 4]) {
      socket.transition({ kind: 'connecting', attempt });
      socket.transition({ kind: 'reconnecting', attempt, nextAttemptAt: Date.now() + 30_000 });
    }
    await vi.advanceTimersByTimeAsync(15_500);
    expect(wrapper.get('[role="status"]').attributes('data-stale')).toBe('true');
    expect(wrapper.text()).toContain('out of date');
    await vi.advanceTimersByTimeAsync(14_500);
    expect(useNodesStore().nodes[0]?.identity.node_label).toBe('Node A');
    expect(useRealtimeStore().stale).toBe(false);
    expect(wrapper.text()).toContain('refreshing periodically');
    const updated = makeNode();
    vi.mocked(apiClient.listNodes).mockResolvedValue([
      { ...updated, identity: { ...updated.identity, node_label: 'REST B' } },
    ]);
    await vi.advanceTimersByTimeAsync(5000);
    expect(useNodesStore().nodes[0]?.identity.node_label).toBe('REST B');
    expect(apiClient.listNodes).toHaveBeenCalledTimes(4);
    socket.transition({ kind: 'open', sinceTs: Date.now() });
    initial('Recovered live');
    await vi.advanceTimersByTimeAsync(60_000);
    expect(apiClient.listNodes).toHaveBeenCalledTimes(4);
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
  });

  it('cancels an in-flight poll and never overwrites a newer WS snapshot', async () => {
    let resolveNodes!: (nodes: ReturnType<typeof makeNode>[]) => void;
    let resolveOverview!: (overview: ReturnType<typeof makeOverview>) => void;
    vi.mocked(apiClient.listNodes).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveNodes = resolve;
      }),
    );
    vi.mocked(apiClient.overview).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveOverview = resolve;
      }),
    );
    await vi.advanceTimersByTimeAsync(500);
    const signal = vi.mocked(apiClient.listNodes).mock.calls[0]?.[0];
    initial('New authoritative state');
    expect(signal?.aborted).toBe(true);
    resolveNodes([]);
    resolveOverview(makeOverview({ total_nodes: 999 }));
    await flushPromises();
    expect(useNodesStore().nodes[0]?.identity.node_label).toBe('New authoritative state');
    expect(useOverviewStore().data?.total_nodes).not.toBe(999);
  });

  it('bounds each request, avoids overlap, pauses when hidden, and cleans up on unmount', async () => {
    vi.mocked(apiClient.listNodes).mockImplementation(
      (signal) =>
        new Promise((_, reject) => {
          signal?.addEventListener('abort', () => reject(new Error('aborted')), { once: true });
        }),
    );
    await vi.advanceTimersByTimeAsync(500);
    await vi.advanceTimersByTimeAsync(9999);
    expect(apiClient.listNodes).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(vi.mocked(apiClient.listNodes).mock.calls[0]?.[0]?.aborted).toBe(true);
    await vi.advanceTimersByTimeAsync(5000);
    expect(apiClient.listNodes).toHaveBeenCalledTimes(2);
    Object.defineProperty(document, 'hidden', { configurable: true, value: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(apiClient.listNodes).toHaveBeenCalledTimes(2);
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(500);
    expect(apiClient.listNodes).toHaveBeenCalledTimes(3);
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(apiClient.listNodes).toHaveBeenCalledTimes(3);
    expect(socket.states.size).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});
