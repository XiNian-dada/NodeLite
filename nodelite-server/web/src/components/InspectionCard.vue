<script setup lang="ts">
import { computed, ref, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import type { InspectionSettingsView } from '@/api';
import DeliveryCheckboxes from './DeliveryCheckboxes.vue';
import NativeDialog from './NativeDialog.vue';

/** Daily inspection editor — compact card overview with modal dialog for detailed settings. */
const inspection = defineModel<InspectionSettingsView>({ required: true });

const { t } = useI18n();
const titleId = useId();
const showModal = ref(false);

const summary = computed(() => {
  const delivery = inspection.value.delivery.length
    ? inspection.value.delivery.map((channel) => t(`alerts.channel.${channel}`)).join(' + ')
    : t('common.not_available');
  return `${inspection.value.local_time || '09:00'} · ${inspection.value.lookback_hours || 24}h · ${delivery}`;
});
</script>

<template>
  <article class="panel channel-card" data-test="inspection-card">
    <header class="card-head">
      <div class="card-title-group">
        <span class="channel-icon">📋</span>
        <div>
          <h2 class="card-title">{{ t('alerts.inspection.title') }}</h2>
          <span class="status-badge" :class="inspection.enabled ? 'status-badge--active' : 'status-badge--disabled'">
            {{ inspection.enabled ? t('alerts.rules.enabled') : t('settings.disabled') }}
          </span>
        </div>
      </div>
      <label class="toggle">
        <input v-model="inspection.enabled" type="checkbox" data-test="inspection-enabled" />
        <span class="toggle-text">{{ t('alerts.inspection.enabled') }}</span>
      </label>
    </header>

    <p v-if="!inspection.enabled" class="collapsed-note" data-test="inspection-collapsed">
      {{ summary }}
    </p>

    <div v-else class="card-body">
      <div class="summary-details">
        <div class="summary-row">
          <span class="summary-label">{{ t('alerts.inspection.schedule') }}:</span>
          <span class="summary-val">{{ inspection.local_time || '09:00' }} · {{ inspection.lookback_hours || 24 }}h</span>
        </div>
        <div class="summary-row">
          <span class="summary-label">{{ t('alerts.inspection.delivery') }}:</span>
          <span class="summary-val">{{ summary.split(' · ')[2] }}</span>
        </div>
      </div>

      <div class="card-actions">
        <button
          type="button"
          class="btn btn--subtle btn--sm"
          data-test="open-inspection-config"
          @click="showModal = true"
        >
          ⚙️ {{ t('alerts.channel.configure') }}
        </button>
      </div>
    </div>

    <!-- Configuration Modal Dialog -->
    <NativeDialog v-if="showModal" :labelled-by="titleId" @close="showModal = false">
      <div class="modal-panel" data-test="inspection-modal">
        <header class="modal-head">
          <div class="modal-title-wrap">
            <span class="modal-icon">📋</span>
            <div>
              <h2 :id="titleId" class="modal-title">{{ t('alerts.inspection.title') }}</h2>
              <p class="modal-subtitle">{{ t('alerts.inspection.thresholds') }}</p>
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

        <div class="form modal-form" data-test="inspection-form">
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

        <footer class="modal-footer">
          <button
            type="button"
            class="btn btn--primary btn--sm"
            data-test="inspection-modal-close"
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
.field input {
  width: 100%;
  background: var(--bg-card-soft);
  color: var(--text-primary);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 9px 10px;
  font: inherit;
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
