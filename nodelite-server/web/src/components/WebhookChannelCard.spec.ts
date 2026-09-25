import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h, reactive } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { viewToDraft, type WebhookDraft } from '@/lib/alertsDraft';
import { makeAlertSettingsView } from '@/api/__fixtures__/nodes';
import WebhookChannelCard from './WebhookChannelCard.vue';

const FAKE_DICT = {
  en: {
    'alerts.secret.keep': 'leave blank to keep',
    'alerts.secret.clear': 'clear',
    'settings.disabled': 'Disabled',
    'alerts.rules.enabled': 'Enabled',
    'alerts.webhook.title': 'Webhook Delivery',
    'alerts.webhook.enabled': 'Enable Webhook',
    'alerts.webhook.url': 'URL',
    'alerts.webhook.secret': 'Secret',
    'alerts.webhook.send_resolved': 'Send resolved',
    'alerts.channel.configure': 'Configure',
    'alerts.channel.status_configured': 'Configured',
    'alerts.channel.status_not_configured': 'Not configured',
    'alerts.modal.done': 'Done',
    'alerts.details': 'Details',
  },
  'zh-CN': {},
};
const Stub = defineComponent({ render: () => h('div') });

function mountCard(webhook: WebhookDraft) {
  return mount(WebhookChannelCard, {
    props: { modelValue: webhook, 'onUpdate:modelValue': () => {} },
    global: { plugins: [getI18n()] },
  });
}

describe('WebhookChannelCard', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve(FAKE_DICT) } as unknown as Response),
    );
    await setupI18n(createApp(Stub));
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
  });

  it('shows the keep-secret placeholder only when a secret is configured in modal', async () => {
    const configured = reactive(
      viewToDraft(makeAlertSettingsView({ webhook: { enabled: true, secret_configured: true } })).webhook,
    );
    const wrapper = mountCard(configured);
    await wrapper.find('[data-test="open-webhook-config"]').trigger('click');
    expect(wrapper.find('[data-test="webhook-secret"]').attributes('placeholder')).toBe(
      'leave blank to keep',
    );
    await wrapper.find('[data-test="webhook-modal-close"]').trigger('click');

    const blank = reactive(
      viewToDraft(makeAlertSettingsView({ webhook: { enabled: true, secret_configured: false } })).webhook,
    );
    const blankWrapper = mountCard(blank);
    await blankWrapper.find('[data-test="open-webhook-config"]').trigger('click');
    expect(blankWrapper.find('[data-test="webhook-secret"]').attributes('placeholder')).toBe('');
  });

  it('binds url + secret edits and the clear flag in modal', async () => {
    const webhook = reactive(viewToDraft(makeAlertSettingsView({ webhook: { enabled: true } })).webhook);
    const wrapper = mountCard(webhook);
    await wrapper.find('[data-test="open-webhook-config"]').trigger('click');
    expect(wrapper.find('[data-test="webhook-modal"]').exists()).toBe(true);

    await wrapper.find('[data-test="webhook-url"]').setValue('https://hooks.example.com/y');
    expect(webhook.url).toBe('https://hooks.example.com/y');
    await wrapper.find('[data-test="webhook-clear-secret"]').setValue(true);
    expect(webhook.clear_secret).toBe(true);
    expect(wrapper.find('[data-test="webhook-secret"]').attributes('disabled')).toBeDefined();
  });

  it('collapses details while disabled and opens config modal', async () => {
    const webhook = reactive(viewToDraft(makeAlertSettingsView({ webhook: { enabled: false } })).webhook);
    const wrapper = mountCard(webhook);

    expect(wrapper.find('[data-test="webhook-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="open-webhook-config"]').exists()).toBe(false);

    await wrapper.find('[data-test="webhook-enabled"]').setValue(true);
    expect(wrapper.find('[data-test="open-webhook-config"]').exists()).toBe(true);

    await wrapper.find('[data-test="open-webhook-config"]').trigger('click');
    expect(wrapper.find('[data-test="webhook-form"]').exists()).toBe(true);
  });
});
