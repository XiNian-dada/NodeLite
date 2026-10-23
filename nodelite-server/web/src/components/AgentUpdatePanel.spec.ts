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
      next_request_at: 0,
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

  it('keeps the last result but disables requests until a failed poll recovers', async () => {
    const wrapper = await panel();
    vi.mocked(apiClient.agentUpdate).mockRejectedValueOnce(new Error('status unavailable'));
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.get('[data-test="agent-update-error"]').text()).toContain('status unavailable');
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe('idle');
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(true);
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.find('[data-test="agent-update-error"]').exists()).toBe(false);
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(false);
  });

  it('explains the cooldown after a confirmed result without implying another upgrade', async () => {
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(view('committed'));
    const wrapper = await panel();
    expect(wrapper.get('[data-test="agent-update-reason"]').text()).toContain('间隔五分钟');
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe(
      'committed',
    );
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(true);
  });

  it('ignores a late status response after navigating to another node', async () => {
    let finishOld!: (result: AgentUpdateView) => void;
    vi.mocked(apiClient.agentUpdate).mockImplementationOnce(
      () => new Promise((resolve) => (finishOld = resolve)),
    );
    const wrapper = await panel();
    expect(wrapper.text()).toContain('等待数据');
    expect(wrapper.find('[data-test="agent-update-version"]').exists()).toBe(false);
    const next = view();
    next.agent_version = '3.0.5-next-node';
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(next);
    await wrapper.setProps({ nodeId: 'jp' });
    await flushPromises();
    finishOld(view('failed'));
    await flushPromises();
    expect(wrapper.get('[data-test="agent-update-version"]').text()).toBe('3.0.5-next-node');
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
