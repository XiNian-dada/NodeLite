import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h, reactive } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { viewToDraft, type AlertsDraft } from '@/lib/alertsDraft';
import { makeAlertSettingsView } from '@/api/__fixtures__/nodes';
import AlertOverviewCard from './AlertOverviewCard.vue';

const FAKE_DICT = {
  en: {
    'alerts.channel.smtp': 'Email',
    'alerts.channel.webhook': 'Webhook',
    'settings.disabled': 'Disabled',
    'alerts.rules.empty_short': 'None',
    'alerts.summary.channels': 'Channels',
    'alerts.summary.rules': 'Rules',
    'alerts.summary.inspection': 'Inspection',
  },
  'zh-CN': {},
};
const Stub = defineComponent({ render: () => h('div') });

function mountCard(draft: AlertsDraft) {
  return mount(AlertOverviewCard, {
    props: { modelValue: draft, 'onUpdate:modelValue': () => {} },
    global: { plugins: [getI18n()] },
  });
}

describe('AlertOverviewCard', () => {
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

  it('summarizes enabled channels and rule counts', () => {
    const draft = reactive(
      viewToDraft(
        makeAlertSettingsView({
          smtp: { enabled: true },
          webhook: { enabled: true },
        }),
      ),
    );
    const text = mountCard(draft).text();
    expect(text).toContain('Email + Webhook');
    // one rule in the fixture, enabled
    expect(text).toContain('1/1');
  });

  it('binds the global enable toggle', async () => {
    const draft = reactive(viewToDraft(makeAlertSettingsView({ enabled: false })));
    const wrapper = mountCard(draft);
    await wrapper.find('[data-test="alerts-enabled"]').setValue(true);
    expect(draft.enabled).toBe(true);
  });
});
