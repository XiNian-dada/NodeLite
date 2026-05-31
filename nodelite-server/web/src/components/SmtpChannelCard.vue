<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import type { AlertSmtpTransport } from '@/api';
import type { SmtpDraft } from '@/lib/alertsDraft';
import CsvField from './CsvField.vue';

/**
 * SMTP channel editor. Binds directly against the parent's reactive draft slice
 * (single source of truth). The stored password is never echoed: when one is on
 * file (`password_configured`), the input shows a "leave blank to keep"
 * placeholder; typing replaces it, the clear checkbox wipes it.
 */
const smtp = defineModel<SmtpDraft>({ required: true });

const { t } = useI18n();

const transports: AlertSmtpTransport[] = ['start_tls', 'tls', 'plain'];
</script>

<template>
  <article class="panel" data-test="smtp-card">
    <header class="card-head">
      <h2 class="card-title">{{ t('alerts.smtp.title') }}</h2>
      <label class="toggle">
        <input v-model="smtp.enabled" type="checkbox" data-test="smtp-enabled" />
        <span>{{ t('alerts.smtp.enabled') }}</span>
      </label>
    </header>

    <div class="form">
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
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 18px 20px;
}
.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
  gap: 12px;
}
.card-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
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
  padding: 8px 10px;
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
}
</style>
