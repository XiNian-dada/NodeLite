<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { AlertsDraft } from '@/lib/alertsDraft';

/** Top card: global alert enable toggle + a read-only summary of the draft. */
const draft = defineModel<AlertsDraft>({ required: true });

const { t } = useI18n();

const channelSummary = computed(() => {
  const enabled = [
    draft.value.smtp.enabled ? t('alerts.channel.smtp') : '',
    draft.value.webhook.enabled ? t('alerts.channel.webhook') : '',
  ].filter(Boolean);
  return enabled.length ? enabled.join(' + ') : t('settings.disabled');
});

const ruleSummary = computed(() => {
  const total = draft.value.rules.length;
  if (!total) return t('alerts.rules.empty_short');
  const enabled = draft.value.rules.filter((rule) => rule.enabled).length;
  return `${enabled}/${total}`;
});

const inspectionSummary = computed(() => {
  const insp = draft.value.inspection;
  if (!insp.enabled) return t('settings.disabled');
  const delivery = insp.delivery.length
    ? insp.delivery.map((c) => t(`alerts.channel.${c}`)).join(' + ')
    : t('common.not_available');
  return `${insp.local_time || '09:00'} · ${insp.lookback_hours || 24}h · ${delivery}`;
});

const tiles = computed(() => [
  { label: t('alerts.summary.channels'), value: channelSummary.value },
  { label: t('alerts.summary.rules'), value: ruleSummary.value },
  { label: t('alerts.summary.inspection'), value: inspectionSummary.value },
]);
</script>

<template>
  <article class="panel" data-test="alert-overview-card">
    <header class="card-head">
      <h2 class="card-title">{{ t('alerts.overview.title') }}</h2>
      <label class="toggle">
        <input v-model="draft.enabled" type="checkbox" data-test="alerts-enabled" />
        <span>{{ t('alerts.overview.enabled') }}</span>
      </label>
    </header>
    <p class="note">{{ t('alerts.overview.note') }}</p>
    <div class="tiles">
      <div v-for="tile in tiles" :key="tile.label" class="tile">
        <span class="tile__label">{{ tile.label }}</span>
        <strong class="tile__value">{{ tile.value }}</strong>
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
  gap: 12px;
  margin-bottom: 8px;
}
.card-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}
.note {
  color: var(--text-muted);
  font-size: 12px;
  margin: 0 0 14px;
}
.tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: 12px;
}
.tile {
  display: flex;
  flex-direction: column;
  gap: 4px;
  background: var(--bg-card-soft);
  border-radius: 10px;
  padding: 10px 12px;
}
.tile__label {
  color: var(--text-muted);
  font-size: 12px;
}
.tile__value {
  color: var(--text-primary);
  font-size: 14px;
  word-break: break-word;
}
.toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-secondary);
}
</style>
