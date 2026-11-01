import { defineStore } from 'pinia';
import { ref, shallowRef } from 'vue';
import {
  apiClient,
  type AlertPreview,
  type AlertSettingsView,
  type UpdateAlertSettingsRequest,
} from '@/api';
import { ApiAbortError } from '@/api/client';
import { useDedupeAsync } from '@/composables/useDedupeAsync';

/**
 * Alert settings (GET/POST /api/settings/alerts). Holds the server's canonical
 * config + preview; the editable draft lives in AlertsView (viewToDraft). Concurrent
 * loads share one request so every caller can wait for the same result. `save` lets
 * its error propagate so the view can surface the server's reauth/validation message.
 */
export const useAlertsStore = defineStore('alerts', () => {
  const config = shallowRef<AlertSettingsView | null>(null);
  const preview = shallowRef<AlertPreview | null>(null);
  const loading = ref(false);
  const saving = ref(false);
  const error = ref<Error | null>(null);

  const loads = useDedupeAsync<'config'>();

  function load(): Promise<void> {
    return loads.run('config', async ({ isCurrent }) => {
      loading.value = true;
      error.value = null;
      try {
        const res = await apiClient.alertSettings();
        if (isCurrent()) {
          config.value = res.config;
          preview.value = res.preview;
        }
      } catch (e) {
        if (e instanceof ApiAbortError) return;
        if (isCurrent()) {
          error.value = e instanceof Error ? e : new Error(String(e));
        }
      } finally {
        if (isCurrent()) {
          loading.value = false;
        }
      }
    });
  }

  function discardPendingLoad(): void {
    loads.abort();
    loading.value = false;
  }

  /** POST the payload; on success refresh config + preview from the response. */
  async function save(payload: UpdateAlertSettingsRequest): Promise<void> {
    saving.value = true;
    // A GET already on the wire, or sent before the POST commits, may carry the
    // pre-save config. The POST response is the newest state, so those must not land.
    discardPendingLoad();
    try {
      const res = await apiClient.updateAlertSettings(payload);
      discardPendingLoad();
      config.value = res.config;
      preview.value = res.preview;
      error.value = null;
    } finally {
      saving.value = false;
    }
  }

  return { config, preview, loading, saving, error, load, save };
});
