import { afterEach, expect, it, vi } from 'vitest';
import { WsClient } from './client';

class Socket {
  static instances: Socket[] = [];
  onopen: (() => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  constructor() {
    Socket.instances.push(this);
  }
  close(): void {
    this.onclose?.({ code: 1006, reason: 'test disconnect' } as CloseEvent);
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  Socket.instances = [];
});

it('keeps one bounded reconnect timer after repeated handshakes and publishes state transitions', async () => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', Socket);
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 200 }));
  Object.defineProperty(document, 'hidden', { configurable: true, value: false });
  const client = new WsClient('ws://localhost/ws/browser', {
    log: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  });
  const states = vi.fn();
  const off = client.onState(states);
  client.connect();
  for (let attempt = 0; attempt < 5; attempt++) {
    const socket = Socket.instances.at(-1)!;
    socket.onerror?.(new Event('error'));
    socket.close();
    const retry = client.getState();
    expect(retry.kind).toBe('reconnecting');
    if (retry.kind !== 'reconnecting') throw new Error('no automatic retry');
    expect(retry.nextAttemptAt - Date.now()).toBeLessThanOrEqual(30_000);
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(Socket.instances).toHaveLength(attempt + 2);
  }
  Socket.instances.at(-1)!.onerror?.(new Event('error'));
  Socket.instances.at(-1)!.close();
  const attempts = Socket.instances.length;
  document.dispatchEvent(new Event('visibilitychange'));
  expect(Socket.instances).toHaveLength(attempts + 1);
  expect(client.getState()).toEqual({ kind: 'connecting', attempt: 1 });
  expect(vi.getTimerCount()).toBe(0);
  Socket.instances.at(-1)!.onopen?.();
  expect(client.getState().kind).toBe('open');
  expect(states).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'open' }));
  off();
  const count = states.mock.calls.length;
  client.destroy();
  expect(states).toHaveBeenCalledTimes(count);
  expect(vi.getTimerCount()).toBe(0);
});
