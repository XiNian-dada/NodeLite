import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import { apiClient } from '@/api';
import type { AgentUpdateView } from '@/api/types';
import dictionary from '../../public/assets/ui-i18n.json';
import AgentUpdatePanel from './AgentUpdatePanel.vue';

vi.mock('@/api', () => ({ apiClient: { agentUpdate: vi.fn(), startAgentUpdate: vi.fn() } }));

function view(phase: AgentUpdateView['status']['phase'] = 'idle'): AgentUpdateView {
  return {
    online: true,
    agent_version: '3.0.5-test.1',
    can_update: phase === 'idle',
    status: {
      capability: 'ready',
      phase,
      request_id: phase === 'idle' ? null : 'a'.repeat(32),
      updated_at: 100,
      installed_sequence: 1,
      target_sequence: null,
      channel: 'stable',
      reason: null,
    },
  };
}

const wrappers: VueWrapper[] = [];
async function panel() {
  const wrapper = mount(AgentUpdatePanel, {
    props: { nodeId: 'hk' },
    global: {
      plugins: [
        createI18n({ legacy: false, locale: 'zh-CN', flatJson: true, messages: dictionary }),
      ],
      stubs: { NativeDialog: { template: '<div><slot /></div>' } },
    },
  });
  wrappers.push(wrapper);
  await flushPromises();
  return wrapper;
}

describe('AgentUpdatePanel', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(view());
    vi.mocked(apiClient.startAgentUpdate).mockResolvedValue(view('queued'));
  });
  afterEach(() => {
    wrappers.splice(0).forEach((wrapper) => wrapper.unmount());
    vi.useRealTimers();
  });

  it('requires explicit confirmation and never treats an accepted request as success', async () => {
    const wrapper = await panel();
    await wrapper.get('[data-test="agent-update-start"]').trigger('click');
    expect(apiClient.startAgentUpdate).not.toHaveBeenCalled();
    await wrapper.get('[data-test="agent-update-confirm-submit"]').trigger('click');
    await flushPromises();
    expect(apiClient.startAgentUpdate).toHaveBeenCalledExactlyOnceWith('hk');
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe('queued');
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(true);
    expect(wrapper.text()).not.toContain('升级已确认成功');
  });

  it('cancellation does not send an update request', async () => {
    const wrapper = await panel();
    await wrapper.get('[data-test="agent-update-start"]').trigger('click');
    await wrapper.get('[data-test="agent-update-cancel"]').trigger('click');
    expect(apiClient.startAgentUpdate).not.toHaveBeenCalled();
  });

  it('disables unenrolled nodes and shows the one-time enable command flag', async () => {
    const disabled = view();
    disabled.can_update = false;
    disabled.status.capability = 'disabled';
    disabled.status.reason = 'not_enrolled';
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(disabled);
    const wrapper = await panel();
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(true);
    expect(wrapper.text()).toContain('--enable-web-updates');
  });

  it('keeps the trial pending while offline and only displays success after confirmation', async () => {
    const trial = view('trial');
    trial.online = false;
    vi.mocked(apiClient.agentUpdate).mockResolvedValueOnce(trial);
    const wrapper = await panel();
    expect(wrapper.text()).toContain('等待重连');
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe('trial');
    const committed = view('committed');
    committed.agent_version = '3.0.5-test.2';
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(committed);
    await vi.advanceTimersByTimeAsync(2000);
    await flushPromises();
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe(
      'committed',
    );
    expect(wrapper.get('[data-test="agent-update-version"]').text()).toBe('3.0.5-test.2');
  });

  it('surfaces request failures without changing the last confirmed result', async () => {
    vi.mocked(apiClient.startAgentUpdate).mockRejectedValue(new Error('request rejected'));
    const wrapper = await panel();
    await wrapper.get('[data-test="agent-update-start"]').trigger('click');
    await wrapper.get('[data-test="agent-update-confirm-submit"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[data-test="agent-update-error"]').text()).toContain('request rejected');
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe('idle');
  });

  it('stops polling on unmount', async () => {
    const wrapper = await panel();
    wrapper.unmount();
    const count = vi.mocked(apiClient.agentUpdate).mock.calls.length;
    await vi.advanceTimersByTimeAsync(10000);
    expect(apiClient.agentUpdate).toHaveBeenCalledTimes(count);
  });
});
