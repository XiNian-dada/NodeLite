import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import { createI18n } from 'vue-i18n';
import TrafficControlStatus from './TrafficControlStatus.vue';

describe('Agent traffic control status', () => {
  it('shows missing capability separately from the requested rate', () => {
    const wrapper = mount(TrafficControlStatus, {
      props: {
        status: {
          state: 'unavailable',
          reason: 'missing_capability',
          desired_rate_kbps: 1000,
          applied_rate_kbps: null,
        },
      },
      global: { plugins: [createI18n({ legacy: false, locale: 'en' })] },
    });
    expect(wrapper.text()).toContain('Unavailable');
    expect(wrapper.text()).toContain('permission is missing');
    expect(wrapper.text()).toContain('Requested: 1000');
    expect(wrapper.text()).not.toContain('Applied: 1000');
  });

  it('reports applied and cleared limits without claiming old Agents support shaping', async () => {
    const wrapper = mount(TrafficControlStatus, {
      global: { plugins: [createI18n({ legacy: false, locale: 'en' })] },
    });
    expect(wrapper.text()).toContain('Not reported');
    await wrapper.setProps({
      status: { state: 'applied', reason: null, desired_rate_kbps: 2000, applied_rate_kbps: 2000 },
    });
    expect(wrapper.text()).toContain('Applied: 2000');
    await wrapper.setProps({
      status: { state: 'applied', reason: null, desired_rate_kbps: null, applied_rate_kbps: null },
    });
    expect(wrapper.text()).toContain('No limit applied');
  });
});
