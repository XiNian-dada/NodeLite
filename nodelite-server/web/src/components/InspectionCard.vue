<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import type { InspectionSettingsView } from '@/api';
import DeliveryCheckboxes from './DeliveryCheckboxes.vue';

/** Daily inspection editor — binds the parent's reactive inspection slice. */
const inspection = defineModel<InspectionSettingsView>({ required: true });

const { t } = useI18n();
</script>

<template>
  <article class="panel" data-test="inspection-card">
    <header class="card-head">
      <h2 class="card-title">{{ t('alerts.inspection.title') }}</h2>
      <label class="toggle">
        <input v-model="inspection.enabled" type="checkbox" data-test="inspection-enabled" />
        <span>{{ t('alerts.inspection.enabled') }}</span>
      </label>
    </header>

    <div class="form">
      <div class="split">
        <label class="field">
          <span>{{ t('alerts.inspection.local_time') }}</span>
          <input v-model="inspection.local_time" type="text" placeholder="09:00" data-test="inspection-local-time" />
        </label>
        <label class="field">
          <span>{{ t('alerts.inspection.lookback_hours') }}</span>
          <input
            v-model.number="inspection.lookback_hours"
            type="number"
            min="1"
            max="720"
            data-test="inspection-lookback"
          />
        </label>
      </div>
      <div class="field">
        <span>{{ t('alerts.inspection.delivery') }}</span>
        <DeliveryCheckboxes v-model="inspection.delivery" />
      </div>
      <div class="split">
        <label class="field">
          <span>{{ t('alerts.inspection.offline_grace_minutes') }}</span>
          <input
            v-model.number="inspection.offline_grace_minutes"
            type="number"
            min="1"
            data-test="inspection-offline-grace"
          />
        </label>
        <label class="field">
          <span>{{ t('alerts.inspection.latency_warn_ms') }}</span>
          <input
            v-model.number="inspection.latency_warn_ms"
            type="number"
            min="1"
            data-test="inspection-latency-warn"
          />
        </label>
      </div>
      <div class="split">
        <label class="field">
          <span>{{ t('alerts.inspection.cpu_warn_percent') }}</span>
          <input
            v-model.number="inspection.cpu_warn_percent"
            type="number"
            min="1"
            max="100"
            data-test="inspection-cpu-warn"
          />
        </label>
        <label class="field">
          <span>{{ t('alerts.inspection.memory_warn_percent') }}</span>
          <input
            v-model.number="inspection.memory_warn_percent"
            type="number"
            min="1"
            max="100"
            data-test="inspection-memory-warn"
          />
        </label>
      </div>
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
.field input {
  width: 100%;
  background: var(--bg-card-soft);
  color: var(--text-primary);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 8px 10px;
  font: inherit;
}
.toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-secondary);
}
</style>
