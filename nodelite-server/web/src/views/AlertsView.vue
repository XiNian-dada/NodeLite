<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppLayout from '@/components/AppLayout.vue';
import AlertOverviewCard from '@/components/AlertOverviewCard.vue';
import SmtpChannelCard from '@/components/SmtpChannelCard.vue';
import WebhookChannelCard from '@/components/WebhookChannelCard.vue';
import InspectionCard from '@/components/InspectionCard.vue';
import RuleList from '@/components/RuleList.vue';
import PreviewCard from '@/components/PreviewCard.vue';
import SettingsMessage from '@/components/SettingsMessage.vue';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import { draftToPayload, emptyAlertsConfig, viewToDraft } from '@/lib/alertsDraft';
import { useAlertsStore } from '@/stores/alerts';

const { t } = useI18n();
const store = useAlertsStore();

// The reactive draft is the single source of truth (no DOM-as-state). Seeded
// from the server config on load and re-seeded after each successful save.
const draft = reactive(emptyAlertsConfig());
const message = reactive<{ state: 'ok' | 'error' | null; text: string }>({ state: null, text: '' });
const deliveryOpen = ref(true);
const inspectionOpen = ref(true);

function seedDraft(): void {
  if (store.config) Object.assign(draft, viewToDraft(store.config));
}

onMounted(async () => {
  await store.load();
  seedDraft();
});

async function save(): Promise<void> {
  message.state = null;
  message.text = t('alerts.saving');
  try {
    await store.save(draftToPayload(draft));
    seedDraft();
    message.state = 'ok';
    message.text = t('alerts.saved');
  } catch (e) {
    if (e instanceof ApiAbortError) {
      message.text = '';
      return;
    }
    message.state = 'error';
    message.text = t('alerts.save_failed', { error: messageFromError(e, 'unknown') });
  }
}
</script>

<template>
  <AppLayout>
    <template #title>
      <h1 class="page-heading">{{ t('alerts.heading') }}</h1>
      <p class="page-subtitle">{{ t('alerts.subtitle') }}</p>
    </template>

    <section class="alerts" data-test="alerts-view">
      <template v-if="store.config">
        <AlertOverviewCard
          :model-value="draft"
          @update:model-value="(next) => Object.assign(draft, next)"
        />

        <div class="alerts__workspace" :class="{ 'alerts__workspace--disabled': !draft.enabled }">
          <div class="alerts__main">
            <section class="alerts-section" data-test="alerts-delivery-section">
              <header class="section-head">
                <div>
                  <h2 class="section-title">{{ t('alerts.section.delivery') }}</h2>
                  <p class="section-note">{{ t('alerts.section.delivery_note') }}</p>
                </div>
                <button
                  type="button"
                  class="btn btn--subtle btn--sm section-toggle"
                  data-test="delivery-toggle"
                  @click="deliveryOpen = !deliveryOpen"
                >
                  {{ deliveryOpen ? t('common.collapse') : t('common.expand') }}
                </button>
              </header>
              <div v-show="deliveryOpen" class="alerts__grid alerts__grid--channels">
                <SmtpChannelCard v-model="draft.smtp" />
                <WebhookChannelCard v-model="draft.webhook" />
              </div>
              <div
                v-show="!deliveryOpen"
                class="section-summary-pill"
                data-test="delivery-collapsed-summary"
                @click="deliveryOpen = true"
              >
                <span class="summary-item">
                  <span class="summary-label">SMTP:</span>
                  <span class="summary-val">{{ draft.smtp.enabled ? (draft.smtp.host || t('alerts.rules.enabled')) : t('settings.disabled') }}</span>
                </span>
                <span class="summary-divider">·</span>
                <span class="summary-item">
                  <span class="summary-label">Webhook:</span>
                  <span class="summary-val">{{ draft.webhook.enabled ? (draft.webhook.url || t('alerts.rules.enabled')) : t('settings.disabled') }}</span>
                </span>
                <span class="summary-expand-hint">{{ t('common.expand') }}</span>
              </div>
            </section>

            <section class="alerts-section" data-test="alerts-inspection-section">
              <header class="section-head">
                <div>
                  <h2 class="section-title">{{ t('alerts.section.inspection') }}</h2>
                  <p class="section-note">{{ t('alerts.section.inspection_note') }}</p>
                </div>
                <button
                  type="button"
                  class="btn btn--subtle btn--sm section-toggle"
                  data-test="inspection-toggle"
                  @click="inspectionOpen = !inspectionOpen"
                >
                  {{ inspectionOpen ? t('common.collapse') : t('common.expand') }}
                </button>
              </header>
              <div v-show="inspectionOpen">
                <InspectionCard v-model="draft.inspection" />
              </div>
              <div
                v-show="!inspectionOpen"
                class="section-summary-pill"
                data-test="inspection-collapsed-summary"
                @click="inspectionOpen = true"
              >
                <span class="summary-item">
                  <span class="summary-label">{{ t('alerts.inspection.title') }}:</span>
                  <span class="summary-val">{{ draft.inspection.enabled ? `${draft.inspection.local_time || '09:00'} · ${draft.inspection.lookback_hours || 24}h` : t('settings.disabled') }}</span>
                </span>
                <span class="summary-expand-hint">{{ t('common.expand') }}</span>
              </div>
            </section>

            <RuleList v-model="draft.rules" />
          </div>

          <aside class="alerts__aside">
            <PreviewCard :preview="store.preview" />
            <article class="save-bar panel" data-test="alerts-save-bar">
              <div class="save-bar__actions">
                <button
                  type="button"
                  class="btn btn--primary"
                  :disabled="store.saving"
                  data-test="alerts-save"
                  @click="save"
                >
                  {{ t('alerts.save') }}
                </button>
                <SettingsMessage :state="message.state" :text="message.text" />
              </div>
            </article>
          </aside>
        </div>
      </template>

      <SettingsMessage
        v-else-if="store.error"
        state="error"
        :text="store.error.message"
        data-test="alerts-error"
      />
      <p v-else class="placeholder" data-test="alerts-loading">
        {{ t('common.waiting_for_data') }}
      </p>
    </section>
  </AppLayout>
</template>

<style scoped>
.alerts {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.alerts__workspace {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(320px, 0.36fr);
  gap: 16px;
  align-items: start;
}
.alerts__workspace--disabled {
  opacity: 0.82;
}
.alerts__main,
.alerts__aside {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.alerts__aside {
  position: sticky;
  top: 16px;
}
.alerts-section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.section-toggle {
  flex-shrink: 0;
}
.section-summary-pill {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  background: var(--bg-card);
  border: 1px dashed var(--border-soft);
  border-radius: var(--radius-md);
  font-size: 13px;
  color: var(--text-muted);
  cursor: pointer;
  transition:
    background-color 0.15s ease,
    border-color 0.15s ease;
}
.section-summary-pill:hover {
  background: var(--bg-card-soft);
  border-color: var(--border-strong);
}
.summary-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.summary-label {
  font-weight: 550;
  color: var(--text-secondary);
}
.summary-val {
  color: var(--text-muted);
  max-width: 280px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.summary-divider {
  color: var(--border-strong);
}
.summary-expand-hint {
  margin-left: auto;
  font-size: 12px;
  color: var(--accent-blue);
  font-weight: 500;
}
.section-title {
  margin: 0;
  font-size: 15px;
  font-weight: 650;
  color: var(--text-primary);
}
.section-note {
  margin: 3px 0 0;
  color: var(--text-muted);
  font-size: 12px;
}
.alerts__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr));
  gap: 16px;
  align-items: start;
}
.alerts__grid--channels {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 16px;
}
.save-bar {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.save-bar :deep(.reauth-fields) {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 12px;
}
.save-bar__actions {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: var(--actions-gap);
}
.page-heading {
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  letter-spacing: 0;
}
.page-subtitle {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 13px;
}
.placeholder {
  color: var(--text-muted);
  font-size: 13px;
}
@media (max-width: 760px) {
  .alerts__workspace,
  .alerts__grid--channels {
    grid-template-columns: 1fr;
  }
  .alerts__aside {
    position: static;
  }
}
</style>
