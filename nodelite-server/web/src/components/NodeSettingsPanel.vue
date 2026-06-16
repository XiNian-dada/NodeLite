<script setup lang="ts">
import { toRef } from 'vue';
import { useI18n } from 'vue-i18n';
import ReauthFields from '@/components/ReauthFields.vue';
import SettingsMessage from '@/components/SettingsMessage.vue';
import { useNodeSettingsDraft } from '@/composables/useNodeSettingsDraft';

/**
 * Per-node settings tab: shows the current node's token info (from the global
 * settings store's agents array) and a refresh-token form with reauth. The
 * server's POST /api/nodes/{id}/refresh-token returns the new expiry; on
 * success, reload the settings store so the token table reflects the change.
 */
const props = defineProps<{ nodeId: string }>();

const { t } = useI18n();
const {
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
} = useNodeSettingsDraft(toRef(props, 'nodeId'), t);
</script>

<template>
  <div class="node-settings" data-test="node-settings-panel">
    <article class="panel" data-test="node-token-info-panel">
      <header class="card-head">
        <h2 class="card-title">{{ t('node.settings.token_info') }}</h2>
      </header>

      <template v-if="agent">
        <div class="info-grid">
          <div class="info-row">
            <span class="info-label">{{ t('node.settings.token_status') }}</span>
            <span class="info-value">{{ expiryLabel }}</span>
          </div>
          <div v-if="expiryDate" class="info-row">
            <span class="info-label">{{ t('node.settings.token_expires_at') }}</span>
            <span class="info-value">{{ expiryDate }}</span>
          </div>
        </div>
      </template>
      <p v-else class="placeholder">
        {{ t('common.waiting_for_data') }}
      </p>
    </article>

    <article class="panel">
      <header class="card-head">
        <h2 class="card-title">{{ t('node.settings.service_meta') }}</h2>
      </header>

      <template v-if="agent">
        <div class="service-form">
          <label class="field">
            <span>{{ t('node.settings.service_expires_at') }}</span>
            <input
              v-model="serviceDraft.serviceDate"
              class="field-input"
              type="date"
              :disabled="serviceDraft.serviceUnlimited"
              data-test="node-service-expiry-input"
            />
          </label>
          <label class="field field--check">
            <span>{{ t('node.settings.service_unlimited') }}</span>
            <span class="check-row">
              <input
                v-model="serviceDraft.serviceUnlimited"
                type="checkbox"
                data-test="node-service-unlimited-input"
              />
              <span>{{ t('node.settings.service_unlimited_hint') }}</span>
            </span>
          </label>
          <label class="field">
            <span>{{ t('node.settings.renewal_price') }}</span>
            <input
              v-model="serviceDraft.renewalPrice"
              class="field-input"
              type="text"
              maxlength="64"
              :placeholder="t('settings.tokens.renewal_price_placeholder')"
              data-test="node-renewal-price-input"
            />
          </label>
          <button
            type="button"
            class="btn btn--primary service-save"
            :disabled="serviceSaving.value"
            data-test="node-service-meta-save"
            @click="saveServiceMetadata"
          >
            {{
              serviceSaving.value
                ? t('node.settings.service_meta_saving')
                : t('node.settings.service_meta_save')
            }}
          </button>
        </div>
        <SettingsMessage :state="serviceMessage.state" :text="serviceMessage.text" />
      </template>
      <p v-else class="placeholder">
        {{ t('common.waiting_for_data') }}
      </p>
    </article>

    <article class="panel">
      <header class="card-head">
        <h2 class="card-title">{{ t('node.settings.location_override') }}</h2>
      </header>

      <template v-if="agent">
        <div class="info-grid location-current">
          <div class="info-row">
            <span class="info-label">{{ t('node.settings.location_auto') }}</span>
            <span class="info-value">{{ automaticLocation }}</span>
          </div>
        </div>
        <div class="location-form">
          <label class="field">
            <span>{{ t('node.settings.location_country') }}</span>
            <input
              v-model="locationDraft.country"
              class="field-input"
              type="text"
              maxlength="64"
              placeholder="HK"
              data-test="node-location-country-input"
            />
          </label>
          <label class="field">
            <span>{{ t('node.settings.location_city') }}</span>
            <input
              v-model="locationDraft.city"
              class="field-input"
              type="text"
              maxlength="64"
              placeholder="Hong Kong"
              data-test="node-location-city-input"
            />
          </label>
          <label class="field">
            <span>{{ t('node.settings.location_latitude') }}</span>
            <input
              v-model="locationDraft.latitude"
              class="field-input"
              type="number"
              step="0.000001"
              min="-90"
              max="90"
              placeholder="22.3193"
              data-test="node-location-latitude-input"
            />
          </label>
          <label class="field">
            <span>{{ t('node.settings.location_longitude') }}</span>
            <input
              v-model="locationDraft.longitude"
              class="field-input"
              type="number"
              step="0.000001"
              min="-180"
              max="180"
              placeholder="114.1694"
              data-test="node-location-longitude-input"
            />
          </label>
          <div class="location-actions">
            <button
              type="button"
              class="btn btn--primary"
              :disabled="locationSaving.value"
              data-test="node-location-save"
              @click="saveLocationOverride()"
            >
              {{
                locationSaving.value
                  ? t('node.settings.location_saving')
                  : t('node.settings.location_save')
              }}
            </button>
            <button
              type="button"
              class="btn"
              :disabled="locationSaving.value"
              data-test="node-location-clear"
              @click="saveLocationOverride(true)"
            >
              {{ t('node.settings.location_clear') }}
            </button>
          </div>
        </div>
        <SettingsMessage :state="locationMessage.state" :text="locationMessage.text" />
      </template>
      <p v-else class="placeholder">
        {{ t('common.waiting_for_data') }}
      </p>
    </article>

    <article class="panel">
      <header class="card-head">
        <h2 class="card-title">{{ t('node.settings.refresh_token') }}</h2>
        <p class="card-note">{{ t('node.settings.refresh_note') }}</p>
      </header>

      <div class="refresh-form">
        <ReauthFields
          v-model:current-password="reauth.current_password"
          v-model:code="reauth.code"
          variant="both"
        />
        <button
          type="button"
          class="btn btn--primary"
          :disabled="saving.value"
          data-test="refresh-token-button"
          @click="refresh"
        >
          {{ t('node.settings.refresh_button') }}
        </button>
        <SettingsMessage :state="message.state" :text="message.text" />
      </div>
    </article>
  </div>
</template>

<style scoped>
.node-settings {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 18px 20px;
}
.card-head {
  margin-bottom: 14px;
}
.card-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.card-note {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 12px;
}
.info-grid {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 10px 12px;
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
}
.info-label {
  font-size: 13px;
  color: var(--text-muted);
}
.info-value {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
}
.placeholder {
  margin: 0;
  color: var(--text-muted);
  font-size: 13px;
}
.refresh-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.service-form {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(140px, 0.65fr) minmax(0, 1fr) auto;
  gap: 12px;
  align-items: end;
}
.location-current {
  margin-bottom: 12px;
}
.location-form {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr)) auto;
  gap: 12px;
  align-items: end;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  color: var(--text-muted);
  font-size: 12px;
}
.field-input {
  width: 100%;
  height: 36px;
  color: var(--text-primary);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  padding: 0 10px;
  font: inherit;
  font-size: 13px;
}
.field-input:focus {
  border-color: var(--border-strong);
  outline: none;
}
.field-input:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
.field--check {
  color: var(--text-muted);
}
.check-row {
  display: inline-flex;
  align-items: center;
  min-height: 36px;
  gap: 8px;
  color: var(--text-secondary);
  font-size: 13px;
}
.check-row input {
  width: 15px;
  height: 15px;
  accent-color: var(--accent-green);
}
.service-save {
  min-height: 36px;
}
.location-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.btn {
  align-self: flex-start;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  padding: 8px 14px;
  font: inherit;
}
.btn--primary {
  color: #fff;
  background: var(--accent-blue);
  border-color: transparent;
}
.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
@media (max-width: 720px) {
  .service-form {
    grid-template-columns: 1fr;
  }
  .location-form {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .location-actions {
    justify-content: flex-start;
  }
}
@media (max-width: 560px) {
  .location-form {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
