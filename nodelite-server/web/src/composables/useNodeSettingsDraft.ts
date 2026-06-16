import { computed, onMounted, reactive, watch, type Ref } from 'vue';
import { apiClient, type SettingsAgentToken } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import { fmtDateTime } from '@/lib/format';
import { useSettingsStore } from '@/stores/settings';

export type NodeSettingsTranslate = (
  key: string,
  named?: Record<string, number | string>,
) => string;

export interface ReauthDraft {
  current_password: string;
  code: string;
}

export interface ServiceDraft {
  serviceDate: string;
  serviceUnlimited: boolean;
  renewalPrice: string;
}

export interface LocationDraft {
  country: string;
  city: string;
  latitude: string;
  longitude: string;
}

export interface SettingsMessageState {
  state: 'ok' | 'error' | null;
  text: string;
}

export function dateInputValue(value: string | null | undefined): string {
  if (!value) return '';
  const ms = Date.parse(value);
  if (Number.isFinite(ms)) return new Date(ms).toISOString().slice(0, 10);
  return /^\d{4}-\d{2}-\d{2}/.test(value) ? value.slice(0, 10) : '';
}

export function serviceExpiresAt(value: string): string | null {
  return value ? `${value}T00:00:00Z` : null;
}

export function optionalNumber(value: string | number): number | null | undefined {
  const trimmed = String(value).trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : undefined;
}

export function reauthBody(reauth: ReauthDraft): { current_password?: string; code?: string } {
  const body: { current_password?: string; code?: string } = {};
  if (reauth.current_password) body.current_password = reauth.current_password;
  if (reauth.code) body.code = reauth.code;
  return body;
}

export function syncDraftsFromAgent(
  agent: SettingsAgentToken | undefined,
  serviceDraft: ServiceDraft,
  locationDraft: LocationDraft,
): void {
  serviceDraft.serviceDate = dateInputValue(agent?.service_expires_at);
  serviceDraft.serviceUnlimited = agent?.service_unlimited ?? false;
  serviceDraft.renewalPrice = agent?.renewal_price ?? '';
  locationDraft.country = agent?.location_override_country ?? '';
  locationDraft.city = agent?.location_override_city ?? '';
  locationDraft.latitude =
    agent?.location_override_latitude == null ? '' : String(agent.location_override_latitude);
  locationDraft.longitude =
    agent?.location_override_longitude == null ? '' : String(agent.location_override_longitude);
}

function resetMessage(message: SettingsMessageState): void {
  message.state = null;
  message.text = '';
}

export function useNodeSettingsDraft(nodeId: Ref<string>, t: NodeSettingsTranslate) {
  const settingsStore = useSettingsStore();

  const reauth = reactive<ReauthDraft>({ current_password: '', code: '' });
  const message = reactive<SettingsMessageState>({ state: null, text: '' });
  const saving = reactive({ value: false });
  const serviceDraft = reactive<ServiceDraft>({
    serviceDate: '',
    serviceUnlimited: false,
    renewalPrice: '',
  });
  const serviceMessage = reactive<SettingsMessageState>({
    state: null,
    text: '',
  });
  const serviceSaving = reactive({ value: false });
  const locationDraft = reactive<LocationDraft>({
    country: '',
    city: '',
    latitude: '',
    longitude: '',
  });
  const locationMessage = reactive<SettingsMessageState>({
    state: null,
    text: '',
  });
  const locationSaving = reactive({ value: false });

  const agent = computed(() => settingsStore.data?.agents.find((a) => a.node_id === nodeId.value));

  onMounted(() => {
    if (!settingsStore.data && !settingsStore.loading) {
      void settingsStore.load();
    }
  });

  const expiryLabel = computed(() => {
    const a = agent.value;
    if (!a) return '—';
    if (!a.token_expires_at) return t('node.settings.token_never_expires');
    const secs = a.token_expires_in_secs;
    if (secs == null || secs < 0) return t('node.settings.token_expired');
    const days = Math.floor(secs / 86400);
    if (days > 0) return t('node.settings.token_expires_in_days', { days });
    const hours = Math.floor(secs / 3600);
    return t('node.settings.token_expires_in_hours', { hours });
  });

  const expiryDate = computed(() => {
    const a = agent.value;
    return a?.token_expires_at ? fmtDateTime(a.token_expires_at) : null;
  });

  const automaticLocation = computed(() => {
    const a = agent.value;
    if (!a) return '—';
    const parts = [a.geoip_city, a.geoip_country].filter(Boolean);
    if (parts.length > 0) return parts.join(', ');
    if (a.geoip_latitude != null && a.geoip_longitude != null) {
      return `${a.geoip_latitude.toFixed(4)}, ${a.geoip_longitude.toFixed(4)}`;
    }
    return '—';
  });

  watch(agent, (value) => syncDraftsFromAgent(value, serviceDraft, locationDraft), {
    immediate: true,
  });

  async function saveServiceMetadata(): Promise<void> {
    resetMessage(serviceMessage);
    serviceSaving.value = true;
    try {
      const renewalPrice = serviceDraft.renewalPrice.trim();
      const resp = await apiClient.updateNodeServiceMetadata(nodeId.value, {
        service_expires_at: serviceDraft.serviceUnlimited
          ? null
          : serviceExpiresAt(serviceDraft.serviceDate),
        service_unlimited: serviceDraft.serviceUnlimited,
        renewal_price: renewalPrice || null,
      });
      await settingsStore.refresh();
      serviceDraft.renewalPrice = renewalPrice;
      serviceMessage.state = 'ok';
      serviceMessage.text = resp.message || t('node.settings.service_meta_saved');
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      serviceMessage.state = 'error';
      serviceMessage.text = t('node.settings.service_meta_failed', {
        error: messageFromError(e, 'unknown'),
      });
    } finally {
      serviceSaving.value = false;
    }
  }

  async function saveLocationOverride(clear = false): Promise<void> {
    resetMessage(locationMessage);
    const latitude = clear ? null : optionalNumber(locationDraft.latitude);
    const longitude = clear ? null : optionalNumber(locationDraft.longitude);
    if (latitude === undefined || longitude === undefined) {
      locationMessage.state = 'error';
      locationMessage.text = t('node.settings.location_invalid_number');
      return;
    }

    locationSaving.value = true;
    try {
      const country = clear ? '' : locationDraft.country.trim();
      const city = clear ? '' : locationDraft.city.trim();
      const resp = await apiClient.updateNodeLocationOverride(nodeId.value, {
        country: country || null,
        city: city || null,
        latitude,
        longitude,
      });
      await settingsStore.refresh();
      locationDraft.country = country;
      locationDraft.city = city;
      if (clear) {
        locationDraft.latitude = '';
        locationDraft.longitude = '';
      }
      locationMessage.state = 'ok';
      locationMessage.text = resp.message || t('node.settings.location_saved');
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      locationMessage.state = 'error';
      locationMessage.text = t('node.settings.location_failed', {
        error: messageFromError(e, 'unknown'),
      });
    } finally {
      locationSaving.value = false;
    }
  }

  async function refresh(): Promise<void> {
    message.state = null;
    message.text = t('node.settings.refreshing');
    saving.value = true;
    try {
      const resp = await apiClient.refreshNodeToken(nodeId.value, reauthBody(reauth));
      await settingsStore.refresh();
      reauth.current_password = '';
      reauth.code = '';
      message.state = 'ok';
      message.text = resp.message || t('node.settings.token_refreshed');
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      message.state = 'error';
      message.text = t('node.settings.refresh_failed', { error: messageFromError(e, 'unknown') });
    } finally {
      saving.value = false;
    }
  }

  return {
    agent,
    automaticLocation,
    expiryDate,
    expiryLabel,
    locationDraft,
    locationMessage,
    locationSaving,
    reauth,
    message,
    refresh,
    saveLocationOverride,
    saveServiceMetadata,
    saving,
    serviceDraft,
    serviceMessage,
    serviceSaving,
  };
}
