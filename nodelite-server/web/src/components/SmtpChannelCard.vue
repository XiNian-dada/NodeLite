<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { AlertSmtpTransport } from '@/api';
import type { SmtpDraft } from '@/lib/alertsDraft';
import CsvField from './CsvField.vue';
import NativeDialog from './NativeDialog.vue';

/**
 * SMTP channel editor. Renders a compact card overview on the main page,
 * with detailed configuration managed through a modal dialog to keep
 * page information density clean and organized.
 */
const smtp = defineModel<SmtpDraft>({ required: true });

const { t } = useI18n();
const titleId = useId();
const showModal = ref(false);

const transports: AlertSmtpTransport[] = ['start_tls', 'tls', 'plain'];
const summary = computed(() => {
  const parts = [
    smtp.value.host || t('settings.disabled'),
    smtp.value.recipients.length ? smtp.value.recipients.join(', ') : '',
  ].filter(Boolean);
  return parts.join(' · ');
});

// Auto-open modal when toggling on an unconfigured channel for smooth onboarding
watch(
  () => smtp.value.enabled,
  (enabled, prev) => {
    if (enabled && !prev && !smtp.value.host) {
      showModal.value = true;
    }
  },
);
</script>

<template>
  <article class="panel channel-card" data-test="smtp-card">
    <header class="card-head">
      <div class="card-title-group">
        <span class="channel-icon">✉️</span>
        <div>
          <h2 class="card-title">{{ t('alerts.smtp.title') }}</h2>
          <span class="status-badge" :class="smtp.enabled ? 'status-badge--active' : 'status-badge--disabled'">
            {{ smtp.enabled ? (smtp.host ? t('alerts.channel.status_configured') : t('alerts.rules.enabled')) : t('settings.disabled') }}
          </span>
        </div>
      </div>
      <label class="toggle">
        <input v-model="smtp.enabled" type="checkbox" data-test="smtp-enabled" />
        <span class="toggle-text">{{ t('alerts.smtp.enabled') }}</span>
      </label>
    </header>

    <p v-if="!smtp.enabled" class="collapsed-note" data-test="smtp-collapsed">
      {{ summary }}
    </p>

    <div v-else class="card-body">
      <div class="summary-details">
        <div class="summary-row">
          <span class="summary-label">{{ t('alerts.smtp.host') }}:</span>
          <span class="summary-val">{{ smtp.host ? `${smtp.host}:${smtp.port || 587}` : t('alerts.channel.status_not_configured') }}</span>
        </div>
        <div v-if="smtp.recipients.length" class="summary-row">
          <span class="summary-label">{{ t('alerts.smtp.recipients') }}:</span>
          <span class="summary-val truncate">{{ smtp.recipients.join(', ') }}</span>
        </div>
      </div>

      <div class="card-actions">
        <button
          type="button"
          class="btn btn--subtle btn--sm"
          data-test="open-smtp-config"
          @click="showModal = true"
        >
          ⚙️ {{ t('alerts.channel.configure') }}
        </button>
      </div>
    </div>

    <!-- Configuration Modal Dialog -->
    <NativeDialog v-if="showModal" :labelled-by="titleId" @close="showModal = false">
      <div class="modal-panel" data-test="smtp-modal">
        <header class="modal-head">
          <div class="modal-title-wrap">
            <span class="modal-icon">✉️</span>
            <div>
              <h2 :id="titleId" class="modal-title">{{ t('alerts.smtp.title') }}</h2>
              <p class="modal-subtitle">{{ t('alerts.details') }}</p>
            </div>
          </div>
          <button
            type="button"
            class="modal-close-icon-btn"
            aria-label="Close"
            @click="showModal = false"
          >
            ✕
          </button>
        </header>

        <div class="form modal-form" data-test="smtp-form">
          <div class="split">
            <label class="field">
              <span>{{ t('alerts.smtp.host') }}</span>
              <input v-model="smtp.host" type="text" data-test="smtp-host" />
            </label>
            <label class="field">
              <span>{{ t('alerts.smtp.port') }}</span>
              <input v-model.number="smtp.port" type="number" min="1" max="65535" data-test="smtp-port" />
            </label>
          </div>
          <label class="field">
            <span>{{ t('alerts.smtp.sender') }}</span>
            <input v-model="smtp.sender" type="text" data-test="smtp-sender" />
          </label>
          <label class="field">
            <span>{{ t('alerts.smtp.recipients') }}</span>
            <CsvField v-model="smtp.recipients" data-test="smtp-recipients" />
          </label>
          <div class="split">
            <label class="field">
              <span>{{ t('alerts.smtp.username') }}</span>
              <input v-model="smtp.username" type="text" data-test="smtp-username" />
            </label>
            <label class="field">
              <span>{{ t('alerts.smtp.transport') }}</span>
              <select v-model="smtp.transport" data-test="smtp-transport">
                <option v-for="value in transports" :key="value" :value="value">
                  {{ t(`alerts.smtp.transport.${value}`) }}
                </option>
              </select>
            </label>
          </div>
          <label class="field">
            <span>{{ t('alerts.smtp.password') }}</span>
            <input
              v-model="smtp.password"
              type="password"
              autocomplete="new-password"
              :placeholder="smtp.password_configured ? t('alerts.secret.keep') : ''"
              :disabled="smtp.clear_password"
              data-test="smtp-password"
            />
          </label>
          <label class="toggle">
            <input v-model="smtp.clear_password" type="checkbox" data-test="smtp-clear-password" />
            <span>{{ t('alerts.secret.clear') }}</span>
          </label>
          <label class="toggle">
            <input v-model="smtp.send_resolved" type="checkbox" data-test="smtp-send-resolved" />
            <span>{{ t('alerts.smtp.send_resolved') }}</span>
          </label>
        </div>

        <footer class="modal-footer">
          <button
            type="button"
            class="btn btn--primary btn--sm"
            data-test="smtp-modal-close"
            @click="showModal = false"
          >
            {{ t('alerts.modal.done') }}
          </button>
        </footer>
      </div>
    </NativeDialog>
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-md);
  padding: 16px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}
.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
  gap: 12px;
}
.card-title-group {
  display: flex;
  align-items: center;
  gap: 10px;
}
.channel-icon {
  font-size: 20px;
}
.card-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary);
}
.status-badge {
  display: inline-block;
  font-size: 11px;
  font-weight: 500;
  margin-top: 2px;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
}
.status-badge--active {
  background: var(--accent-green-soft);
  color: var(--accent-green);
}
.status-badge--disabled {
  background: var(--bg-card-soft);
  color: var(--text-muted);
}
.toggle-text {
  font-size: 12px;
}
.collapsed-note {
  margin: 0;
  background: var(--bg-card-soft);
  border: 1px dashed var(--border-soft);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: 13px;
  padding: 10px 12px;
}
.card-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.summary-details {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
}
.summary-row {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
}
.summary-label {
  font-weight: 500;
  color: var(--text-secondary);
}
.summary-val {
  color: var(--text-primary);
}
.truncate {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 260px;
}
.card-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 4px;
}

/* Modal layout styles */
.modal-panel {
  position: fixed;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: calc(100% - 32px);
  max-width: 520px;
  max-height: calc(100vh - 64px);
  overflow-y: auto;
  background: var(--bg-card);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  z-index: 100;
}
.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--border-soft);
  padding-bottom: 12px;
}
.modal-title-wrap {
  display: flex;
  align-items: center;
  gap: 10px;
}
.modal-icon {
  font-size: 22px;
}
.modal-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
}
.modal-subtitle {
  margin: 2px 0 0;
  font-size: 12px;
  color: var(--text-muted);
}
.modal-close-icon-btn {
  background: transparent;
  border: none;
  color: var(--text-muted);
  font-size: 16px;
  cursor: pointer;
  padding: 4px;
  border-radius: var(--radius-sm);
}
.modal-close-icon-btn:hover {
  color: var(--text-primary);
  background: var(--bg-card-soft);
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  border-top: 1px solid var(--border-soft);
  padding-top: 12px;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.split {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 13px;
  color: var(--text-muted);
}
.field input,
.field select {
  width: 100%;
  background: var(--bg-card-soft);
  color: var(--text-primary);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 9px 10px;
  font: inherit;
}
.field input:disabled {
  opacity: 0.5;
}
.toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-secondary);
  cursor: pointer;
}
@media (max-width: 560px) {
  .card-head,
  .split {
    grid-template-columns: 1fr;
  }
}
</style>
