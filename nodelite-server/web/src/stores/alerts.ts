import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import {
  apiClient,
  type AlertPreview,
  type AlertSettingsView,
  type UpdateAlertSettingsRequest,
} from '@/api';
import { ApiAbortError } from '@/api/client';

/**
 * Alert settings (GET/POST /api/settings/alerts). Holds the server's canonical
 * config + preview; the editable draft lives in AlertsView (viewToDraft). Single
 * global resource, so a simple concurrent-load guard suffices. `save` lets its
 * error propagate so the view can surface the server's reauth/validation message.
 */
export const useAlertsStore = defineStore('alerts', () => {
  const config = shallowRef<AlertSettingsView | null>(null);
  const preview = shallowRef<AlertPreview | null>(null);
  const loading = ref(false);
  const saving = ref(false);
  const error = ref<Error | null>(null);

  async function load(): Promise<void> {
    if (loading.value) return;
    loading.value = true;
    error.value = null;
    try {
      const res = await apiClient.alertSettings();
      config.value = res.config;
      preview.value = res.preview;
    } catch (e) {
      if (e instanceof ApiAbortError) return;
      error.value = e instanceof Error ? e : new Error(String(e));
    } finally {
      loading.value = false;
    }
  }

  /** POST the payload; on success refresh config + preview from the response. */
  async function save(payload: UpdateAlertSettingsRequest): Promise<void> {
    saving.value = true;
    try {
      const res = await apiClient.updateAlertSettings(payload);
      config.value = res.config;
      preview.value = res.preview;
    } finally {
      saving.value = false;
    }
  }

  return { config, preview, loading, saving, error, load, save };
});
