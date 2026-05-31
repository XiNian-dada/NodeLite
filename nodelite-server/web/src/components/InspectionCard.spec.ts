import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { createApp, defineComponent, h, reactive } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import type { InspectionSettingsView } from '@/api';
import { viewToDraft } from '@/lib/alertsDraft';
import { makeAlertSettingsView } from '@/api/__fixtures__/nodes';
import InspectionCard from './InspectionCard.vue';

const FAKE_DICT = {
  en: { 'alerts.channel.smtp': 'Email', 'alerts.channel.webhook': 'Webhook' },
  'zh-CN': {},
};
const Stub = defineComponent({ render: () => h('div') });

function mountCard(inspection: InspectionSettingsView) {
  return mount(InspectionCard, {
    props: { modelValue: inspection, 'onUpdate:modelValue': () => {} },
    global: { plugins: [getI18n()] },
  });
}

describe('InspectionCard', () => {
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

  it('binds numeric fields back as numbers, not strings', async () => {
    const inspection = reactive(viewToDraft(makeAlertSettingsView()).inspection);
    const wrapper = mountCard(inspection);
    await wrapper.find('[data-test="inspection-lookback"]').setValue('48');
    expect(inspection.lookback_hours).toBe(48);
    expect(typeof inspection.lookback_hours).toBe('number');
    await wrapper.find('[data-test="inspection-cpu-warn"]').setValue('70');
    expect(inspection.cpu_warn_percent).toBe(70);
  });

  it('toggles delivery channels through DeliveryCheckboxes', async () => {
    const inspection = reactive(
      viewToDraft(makeAlertSettingsView({ inspection: { delivery: ['smtp'] } })).inspection,
    );
    const wrapper = mountCard(inspection);
    await wrapper.find('[data-test="delivery-webhook"]').setValue(true);
    expect(inspection.delivery).toEqual(['smtp', 'webhook']);
  });
});
