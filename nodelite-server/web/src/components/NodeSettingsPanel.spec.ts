import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createApp, defineComponent, h } from 'vue';
import { setupI18n, getI18n, __resetI18nForTest } from '@/i18n';
import { apiClient } from '@/api';
import { makeSettings } from '@/api/__fixtures__/nodes';
import NodeSettingsPanel from './NodeSettingsPanel.vue';

vi.mock('@/api', async () => {
  const actual = await vi.importActual<typeof import('@/api')>('@/api');
  return {
    ...actual,
    apiClient: {
      ...actual.apiClient,
      settings: vi.fn(),
      refreshNodeToken: vi.fn(),
      updateNodeServiceMetadata: vi.fn(),
      updateNodeLocationOverride: vi.fn(),
    },
  };
});

const mockSettings = vi.mocked(apiClient.settings);
const mockRefresh = vi.mocked(apiClient.refreshNodeToken);
const mockUpdateMeta = vi.mocked(apiClient.updateNodeServiceMetadata);
const mockUpdateLocation = vi.mocked(apiClient.updateNodeLocationOverride);

const FAKE_DICT = {
  en: {
    'node.settings.token_info': 'Token Info',
    'node.settings.token_status': 'Status',
    'node.settings.token_expires_at': 'Expires at',
    'node.settings.token_never_expires': 'Never expires',
    'node.settings.token_expired': 'Expired',
    'node.settings.token_expires_in_days': '{days} days',
    'node.settings.token_expires_in_hours': '{hours} hours',
    'node.settings.service_meta': 'Service Renewal',
    'node.settings.service_expires_at': 'Service expiry',
    'node.settings.service_unlimited': 'Unlimited',
    'node.settings.service_unlimited_hint': 'No limit',
    'node.settings.renewal_price': 'Renewal price',
    'node.settings.traffic_limit': 'Monthly traffic limit (GiB)',
    'node.settings.traffic_limit_placeholder': '1024',
    'node.settings.traffic_accounting': 'Traffic accounting',
    'node.settings.traffic_accounting_bidirectional': 'Bidirectional',
    'node.settings.traffic_accounting_inbound': 'Inbound',
    'node.settings.traffic_accounting_outbound': 'Outbound',
    'node.settings.traffic_throttle': 'Limit rate',
    'node.settings.traffic_throttle_placeholder': '10',
    'node.settings.traffic_throttle_hint': 'Applied after 95%',
    'node.settings.traffic_invalid_number': 'Invalid traffic value',
    'node.settings.traffic_throttle_requires_limit': 'Limit required',
    'node.settings.service_meta_save': 'Save',
    'node.settings.service_meta_saving': 'Saving…',
    'node.settings.service_meta_saved': 'Saved',
    'node.settings.service_meta_failed': 'Save failed: {error}',
    'node.settings.location_override': 'Manual Location',
    'node.settings.location_auto': 'Auto detected',
    'node.settings.location_country': 'Country / region',
    'node.settings.location_city': 'City',
    'node.settings.location_latitude': 'Latitude',
    'node.settings.location_longitude': 'Longitude',
    'node.settings.location_save': 'Save',
    'node.settings.location_saving': 'Saving…',
    'node.settings.location_clear': 'Clear',
    'node.settings.location_saved': 'Location saved',
    'node.settings.location_failed': 'Location save failed: {error}',
    'node.settings.location_invalid_number': 'Latitude and longitude must be valid numbers.',
    'node.settings.refresh_token': 'Refresh Token',
    'node.settings.refresh_note': 'Generate a new token for this node',
    'node.settings.refresh_button': 'Refresh',
    'node.settings.refreshing': 'Refreshing…',
    'node.settings.token_refreshed': 'Token refreshed',
    'node.settings.refresh_failed': 'Refresh failed: {error}',
    'common.waiting_for_data': 'Waiting…',
    'settings.password.current': 'Current password',
    'settings.security.verification_code': 'Code',
    'settings.tokens.renewal_price_placeholder': '$5/mo',
  },
  'zh-CN': {},
};

const Stub = defineComponent({ render: () => h('div') });

describe('NodeSettingsPanel', () => {
  beforeEach(async () => {
    __resetI18nForTest();
    mockSettings.mockResolvedValue(
      makeSettings({
        agents: [
          {
            node_id: 'node-a',
            node_label: 'Node A',
            online: true,
            agent_version: '1.0.0',
            remote_ip: '10.0.0.1',
            tags: [],
            token_expires_at: '2026-06-15T00:00:00Z',
            token_expires_in_secs: 1296000, // 15 days
            service_expires_at: '2026-12-31T00:00:00Z',
            service_unlimited: false,
            renewal_price: '$4/mo',
            traffic_limit_bytes: null,
            traffic_accounting: 'bidirectional',
            traffic_throttle_kbps: null,
            geoip_country: 'CN',
            geoip_city: 'Shenyang',
            geoip_latitude: 41.8057,
            geoip_longitude: 123.4315,
            location_override_country: null,
            location_override_city: null,
            location_override_latitude: null,
            location_override_longitude: null,
          },
          {
            node_id: 'node-b',
            node_label: 'Node B',
            online: false,
            agent_version: null,
            remote_ip: null,
            tags: [],
            token_expires_at: null,
            token_expires_in_secs: null,
            service_expires_at: null,
            service_unlimited: false,
            renewal_price: null,
            traffic_limit_bytes: null,
            traffic_accounting: 'bidirectional',
            traffic_throttle_kbps: null,
            geoip_country: null,
            geoip_city: null,
            geoip_latitude: null,
            geoip_longitude: null,
            location_override_country: null,
            location_override_city: null,
            location_override_latitude: null,
            location_override_longitude: null,
          },
        ],
      }),
    );
    mockRefresh.mockResolvedValue({
      ok: true,
      message: 'Token refreshed successfully',
      token_expires_at: '2026-07-01T00:00:00Z',
      token_expires_in_secs: 2592000,
    });
    mockUpdateMeta.mockResolvedValue({
      ok: true,
      message: 'Saved',
    });
    mockUpdateLocation.mockResolvedValue({
      ok: true,
      message: 'Location saved',
    });
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve(FAKE_DICT),
      } as unknown as Response),
    );
    await setupI18n(createApp(Stub));
  });

  afterEach(() => {
    __resetI18nForTest();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  async function mountPanel(nodeId: string, options: { preload?: boolean } = {}) {
    const pinia = createPinia();
    setActivePinia(pinia);
    const { useSettingsStore } = await import('@/stores/settings');
    const store = useSettingsStore();
    if (options.preload !== false) {
      await store.load();
    }
    const wrapper = mount(NodeSettingsPanel, {
      props: { nodeId },
      global: { plugins: [pinia, getI18n()] },
    });
    await flushPromises();
    return wrapper;
  }

  it('loads token info when the settings store is empty', async () => {
    const wrapper = await mountPanel('node-a', { preload: false });
    const rows = wrapper.find('[data-test="node-token-info-panel"]').findAll('.info-row');

    expect(mockSettings).toHaveBeenCalledTimes(1);
    expect(rows).toHaveLength(2);
    expect(rows[0]?.text()).toContain('15 days');
  });

  it('renders token info for the matched node', async () => {
    const wrapper = await mountPanel('node-a');
    expect(wrapper.find('[data-test="node-settings-panel"]').exists()).toBe(true);
    const rows = wrapper.find('[data-test="node-token-info-panel"]').findAll('.info-row');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.text()).toContain('Status');
    expect(rows[0]?.text()).toContain('15 days');
    expect(rows[1]?.text()).toContain('Expires at');
  });

  it('shows "never expires" when token_expires_at is null', async () => {
    const wrapper = await mountPanel('node-b');
    const rows = wrapper.find('[data-test="node-token-info-panel"]').findAll('.info-row');
    expect(rows).toHaveLength(1);
    expect(rows[0]?.text()).toContain('Never expires');
  });

  it('refreshes the token with reauth and shows success message', async () => {
    const wrapper = await mountPanel('node-a');
    await wrapper.find('[data-test="reauth-password"]').setValue('hunter2');
    await wrapper.find('[data-test="refresh-token-button"]').trigger('click');
    await flushPromises();

    expect(mockRefresh).toHaveBeenCalledTimes(1);
    expect(mockRefresh.mock.calls[0]?.[0]).toBe('node-a');
    expect(mockRefresh.mock.calls[0]?.[1]).toMatchObject({ current_password: 'hunter2' });
    expect(mockSettings).toHaveBeenCalledTimes(2); // initial load + refresh after success
    expect(wrapper.find('[data-test="settings-message"]').text()).toBe('Token refreshed successfully');
  });

  it('saves editable service expiry and renewal price', async () => {
    const wrapper = await mountPanel('node-a');
    await wrapper.find('[data-test="node-service-expiry-input"]').setValue('2027-01-15');
    await wrapper.find('[data-test="node-renewal-price-input"]').setValue('  $5/mo  ');
    await wrapper.find('[data-test="node-service-meta-save"]').trigger('click');
    await flushPromises();

    expect(mockUpdateMeta).toHaveBeenCalledWith('node-a', {
      service_expires_at: '2027-01-15T00:00:00Z',
      service_unlimited: false,
      renewal_price: '$5/mo',
      traffic_limit_bytes: null,
      traffic_accounting: 'bidirectional',
      traffic_throttle_kbps: null,
    });
    expect(mockSettings).toHaveBeenCalledTimes(2);
    expect(wrapper.find('[data-test="settings-message"]').text()).toBe('Saved');
  });

  it('saves unlimited service metadata from the node tab', async () => {
    const wrapper = await mountPanel('node-a');
    await wrapper.find('[data-test="node-service-expiry-input"]').setValue('2027-01-15');
    await wrapper.find('[data-test="node-service-unlimited-input"]').setValue(true);
    await wrapper.find('[data-test="node-service-meta-save"]').trigger('click');
    await flushPromises();

    expect(mockUpdateMeta).toHaveBeenCalledWith('node-a', {
      service_expires_at: null,
      service_unlimited: true,
      renewal_price: '$4/mo',
      traffic_limit_bytes: null,
      traffic_accounting: 'bidirectional',
      traffic_throttle_kbps: null,
    });
  });

  it('saves a directional traffic budget and its optional throttle', async () => {
    const wrapper = await mountPanel('node-a');
    await wrapper.find('[data-test="node-traffic-limit-input"]').setValue('1024');
    await wrapper.find('[data-test="node-traffic-accounting-input"]').setValue('outbound');
    await wrapper.find('[data-test="node-traffic-throttle-input"]').setValue('10');
    await wrapper.find('[data-test="node-service-meta-save"]').trigger('click');
    await flushPromises();

    expect(mockUpdateMeta).toHaveBeenCalledWith('node-a', {
      service_expires_at: '2026-12-31T00:00:00Z',
      service_unlimited: false,
      renewal_price: '$4/mo',
      traffic_limit_bytes: 1_099_511_627_776,
      traffic_accounting: 'outbound',
      traffic_throttle_kbps: 10_000,
    });
  });

  it('saves a manual location override from the node tab', async () => {
    const wrapper = await mountPanel('node-a');
    expect(wrapper.text()).toContain('Shenyang, CN');

    await wrapper.find('[data-test="node-location-country-input"]').setValue(' HK ');
    await wrapper.find('[data-test="node-location-city-input"]').setValue(' Hong Kong ');
    await wrapper.find('[data-test="node-location-latitude-input"]').setValue('22.3193');
    await wrapper.find('[data-test="node-location-longitude-input"]').setValue('114.1694');
    await wrapper.find('[data-test="node-location-save"]').trigger('click');
    await flushPromises();

    expect(mockUpdateLocation).toHaveBeenCalledWith('node-a', {
      country: 'HK',
      city: 'Hong Kong',
      latitude: 22.3193,
      longitude: 114.1694,
    });
    expect(mockSettings).toHaveBeenCalledTimes(2);
    expect(wrapper.find('[data-test="settings-message"]').text()).toBe('Location saved');
  });

  it('surfaces the server error message when refresh fails', async () => {
    const { ApiError } = await import('@/api/client');
    mockRefresh.mockReset();
    mockRefresh.mockRejectedValueOnce(
      new ApiError(400, JSON.stringify({ ok: false, message: 'invalid password' })),
    );
    const wrapper = await mountPanel('node-a');
    await wrapper.find('[data-test="refresh-token-button"]').trigger('click');
    await flushPromises();

    const msg = wrapper.find('[data-test="settings-message"]');
    expect(msg.classes()).toContain('error');
    expect(msg.text()).toBe('Refresh failed: invalid password');
  });
});
