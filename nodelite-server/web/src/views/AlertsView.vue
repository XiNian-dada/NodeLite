<script setup lang="ts">
import { onMounted, reactive } from 'vue';
import { useI18n } from 'vue-i18n';
import AppLayout from '@/components/AppLayout.vue';
import AlertOverviewCard from '@/components/AlertOverviewCard.vue';
import SmtpChannelCard from '@/components/SmtpChannelCard.vue';
import WebhookChannelCard from '@/components/WebhookChannelCard.vue';
import InspectionCard from '@/components/InspectionCard.vue';
import RuleList from '@/components/RuleList.vue';
import PreviewCard from '@/components/PreviewCard.vue';
import ReauthFields from '@/components/ReauthFields.vue';
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
// Both reauth fields always show (matches legacy); the server validates whichever
// applies given the account's 2FA state. draftToPayload omits blanks.
const reauth = reactive({ current_password: '', code: '' });
const message = reactive<{ state: 'ok' | 'error' | null; text: string }>({ state: null, text: '' });

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
    await store.save(draftToPayload(draft, reauth));
    seedDraft();
    reauth.current_password = '';
    reauth.code = '';
    message.state = 'ok';
    message.text = t('alerts.saved');
  } catch (e) {
    if (e instanceof ApiAbortError) return;
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
              </header>
              <div class="alerts__grid alerts__grid--channels">
                <SmtpChannelCard v-model="draft.smtp" />
                <WebhookChannelCard v-model="draft.webhook" />
              </div>
            </section>

            <section class="alerts-section" data-test="alerts-inspection-section">
              <header class="section-head">
                <div>
                  <h2 class="section-title">{{ t('alerts.section.inspection') }}</h2>
                  <p class="section-note">{{ t('alerts.section.inspection_note') }}</p>
                </div>
              </header>
              <InspectionCard v-model="draft.inspection" />
            </section>

            <RuleList v-model="draft.rules" />
          </div>

          <aside class="alerts__aside">
            <PreviewCard :preview="store.preview" />
            <article class="save-bar panel" data-test="alerts-save-bar">
              <ReauthFields
                v-model:current-password="reauth.current_password"
                v-model:code="reauth.code"
                variant="both"
              />
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
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
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
  align-items: flex-start;
  gap: 10px;
}
.btn {
  align-self: flex-start;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
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
