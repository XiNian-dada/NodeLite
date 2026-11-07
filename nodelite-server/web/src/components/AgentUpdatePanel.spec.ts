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
    status_reported: true,
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
async function panel(locale = 'zh-CN') {
  const wrapper = mount(AgentUpdatePanel, {
    props: { nodeId: 'hk' },
    global: {
      plugins: [createI18n({ legacy: false, locale, flatJson: true, messages: dictionary })],
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

  it.each([
    ['ready', null, 'enabled', '已启用'],
    ['ready', 'rate_limited', 'enabled', '已启用'],
    ['ready', 'update_failed', 'enabled', '已启用'],
    ['disabled', 'busy', 'enabled', '已启用'],
    ['disabled', 'not_enrolled', 'not_enabled', '未启用'],
    ['disabled', 'disabled_by_policy', 'not_enabled', '未启用'],
    ['unsupported', 'unsupported_environment', 'unsupported', '不支持'],
    ['disabled', 'unsupported_environment', 'unsupported', '不支持'],
    ['disabled', 'local_state_invalid', 'needs_attention', '需要检查'],
    ['manual_recovery', 'manual_recovery', 'needs_attention', '需要检查'],
    ['disabled', 'manual_recovery', 'needs_attention', '需要检查'],
    ['disabled', null, 'unknown', '暂时无法确认'],
  ] as const)(
    'shows enrollment for %s/%s independently of button availability',
    async (capability, reason, state, label) => {
      const result = view();
      result.can_update = false;
      result.status.capability = capability;
      result.status.reason = reason;
      vi.mocked(apiClient.agentUpdate).mockResolvedValue(result);
      const wrapper = await panel();
      const badge = wrapper.get('[data-test="agent-update-enrollment"]');
      expect(badge.attributes('data-enrollment')).toBe(state);
      expect(badge.text()).toBe(label);
      expect(wrapper.find('[data-test="agent-update-enable-help"]').exists()).toBe(
        state === 'not_enabled',
      );
    },
  );

  it('does not treat the server default or a previous connection report as confirmed enrollment', async () => {
    const unreported = view();
    unreported.status_reported = false;
    unreported.can_update = false;
    unreported.status.capability = 'disabled';
    unreported.status.reason = 'not_enrolled';
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(unreported);
    const wrapper = await panel();
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('暂时无法确认');
    expect(wrapper.get('[data-test="agent-update-reason"]').text()).toContain('等待 Agent 上报');
    expect(wrapper.get('[data-test="agent-update-enable-help"]').attributes('href')).toBe(
      'https://xinian-dada.github.io/NodeLite/upgrades.html',
    );
    unreported.status.capability = 'ready';
    unreported.status.reason = null;
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('暂时无法确认');
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(view());
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('已启用');
  });

  it('updates the badge when the node enables web updates', async () => {
    const disabled = view();
    disabled.can_update = false;
    disabled.status.capability = 'disabled';
    disabled.status.reason = 'disabled_by_policy';
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(disabled);
    const wrapper = await panel();
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('未启用');
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(view());
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('已启用');
    expect(wrapper.find('[data-test="agent-update-enable-help"]').exists()).toBe(false);
  });

  it('shows the enrollment label in English', async () => {
    const wrapper = await panel('en');
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('Enabled');
    expect(wrapper.text()).toContain('Web updates');
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
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('暂时无法确认');
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
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('暂时无法确认');
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe('idle');
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(true);
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.find('[data-test="agent-update-error"]').exists()).toBe(false);
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('已启用');
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

  it('shows current local work instead of a past web success while the node is busy', async () => {
    const local = view('committed');
    local.status.capability = 'disabled';
    local.status.reason = 'busy';
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(local);
    const wrapper = await panel();
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe(
      'local_busy',
    );
    expect(wrapper.text()).toContain('节点已在处理一次升级');
    expect(wrapper.text()).not.toContain('升级已确认成功');
    expect(
      wrapper.get<HTMLButtonElement>('[data-test="agent-update-start"]').element.disabled,
    ).toBe(true);
    vi.mocked(apiClient.agentUpdate).mockResolvedValue(view('committed'));
    await vi.advanceTimersByTimeAsync(2000);
    expect(wrapper.get('[data-test="agent-update-phase"]').attributes('data-phase')).toBe(
      'committed',
    );
  });

  it('ignores a late status response after navigating to another node', async () => {
    let finishOld!: (result: AgentUpdateView) => void;
    vi.mocked(apiClient.agentUpdate).mockImplementationOnce(
      () => new Promise((resolve) => (finishOld = resolve)),
    );
    const wrapper = await panel();
    expect(wrapper.text()).toContain('等待数据');
    expect(wrapper.get('[data-test="agent-update-enrollment"]').text()).toBe('暂时无法确认');
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
