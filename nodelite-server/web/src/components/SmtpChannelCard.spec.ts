import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h, reactive } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { viewToDraft, type SmtpDraft } from '@/lib/alertsDraft';
import { makeAlertSettingsView } from '@/api/__fixtures__/nodes';
import SmtpChannelCard from './SmtpChannelCard.vue';

const FAKE_DICT = {
  en: {
    'alerts.secret.keep': 'leave blank to keep',
    'alerts.secret.clear': 'clear',
    'settings.disabled': 'Disabled',
    'alerts.rules.enabled': 'Enabled',
    'alerts.smtp.title': 'SMTP Delivery',
    'alerts.smtp.enabled': 'Enable SMTP',
    'alerts.smtp.host': 'Host',
    'alerts.smtp.port': 'Port',
    'alerts.smtp.sender': 'Sender',
    'alerts.smtp.recipients': 'Recipients',
    'alerts.smtp.username': 'Username',
    'alerts.smtp.transport': 'Transport',
    'alerts.smtp.password': 'Password',
    'alerts.smtp.send_resolved': 'Send resolved',
    'alerts.channel.configure': 'Configure',
    'alerts.channel.status_configured': 'Configured',
    'alerts.channel.status_not_configured': 'Not configured',
    'alerts.modal.done': 'Done',
    'alerts.details': 'Details',
  },
  'zh-CN': {},
};
const Stub = defineComponent({ render: () => h('div') });

function mountCard(smtp: SmtpDraft) {
  return mount(SmtpChannelCard, {
    props: { modelValue: smtp, 'onUpdate:modelValue': () => {} },
    global: { plugins: [getI18n()] },
  });
}

describe('SmtpChannelCard', () => {
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

  it('renders card overview and opens modal to inspect draft values and placeholder', async () => {
    const smtp = reactive(viewToDraft(makeAlertSettingsView()).smtp);
    const wrapper = mountCard(smtp);
    expect(wrapper.find('[data-test="smtp-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="open-smtp-config"]').exists()).toBe(true);

    // Open configuration modal
    await wrapper.find('[data-test="open-smtp-config"]').trigger('click');
    expect(wrapper.find('[data-test="smtp-modal"]').exists()).toBe(true);
    expect((wrapper.find('[data-test="smtp-host"]').element as HTMLInputElement).value).toBe('smtp.example.com');
    expect(wrapper.find('[data-test="smtp-password"]').attributes('placeholder')).toBe('leave blank to keep');

    // Close modal
    await wrapper.find('[data-test="smtp-modal-close"]').trigger('click');
    expect(wrapper.find('[data-test="smtp-modal"]').exists()).toBe(false);
  });

  it('binds enabled + password edits back into the draft slice', async () => {
    const smtp = reactive(viewToDraft(makeAlertSettingsView({ smtp: { enabled: false } })).smtp);
    const wrapper = mountCard(smtp);
    expect(wrapper.find('[data-test="smtp-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="open-smtp-config"]').exists()).toBe(false);

    await wrapper.find('[data-test="smtp-enabled"]').setValue(true);
    expect(smtp.enabled).toBe(true);
    expect(wrapper.find('[data-test="open-smtp-config"]').exists()).toBe(true);

    await wrapper.find('[data-test="open-smtp-config"]').trigger('click');
    expect(wrapper.find('[data-test="smtp-form"]').exists()).toBe(true);
    await wrapper.find('[data-test="smtp-password"]').setValue('new-secret');
    expect(smtp.password).toBe('new-secret');
  });

  it('clear-password disables the password input and sets the flag', async () => {
    const smtp = reactive(viewToDraft(makeAlertSettingsView()).smtp);
    const wrapper = mountCard(smtp);
    await wrapper.find('[data-test="open-smtp-config"]').trigger('click');
    await wrapper.find('[data-test="smtp-clear-password"]').setValue(true);
    expect(smtp.clear_password).toBe(true);
    expect(wrapper.find('[data-test="smtp-password"]').attributes('disabled')).toBeDefined();
  });

  it('edits recipients through CsvField', async () => {
    const smtp = reactive(viewToDraft(makeAlertSettingsView()).smtp);
    const wrapper = mountCard(smtp);
    await wrapper.find('[data-test="open-smtp-config"]').trigger('click');
    await wrapper.find('[data-test="smtp-recipients"]').setValue('a@x.com, b@x.com');
    expect(smtp.recipients).toEqual(['a@x.com', 'b@x.com']);
  });
});
