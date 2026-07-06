<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppLayout from '@/components/AppLayout.vue';
import { apiClient } from '@/api';
import type { AuditLogEntry } from '@/api';
import { useLanguage } from '@/i18n/language';
import { fmtDateTime } from '@/lib/format';

const auditLogs = ref<AuditLogEntry[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const { t } = useI18n();
const { currentLocale } = useLanguage();

const formattedAuditLogs = computed(() => {
  return auditLogs.value.map((entry) => ({
    ...entry,
    formattedTime: fmtDateTime(entry.timestamp, currentLocale.value),
    eventLabel: formatEventType(entry.event_type),
    statusClass: entry.success ? 'status-success' : 'status-failure',
  }));
});

function formatEventType(type: string): string {
  const key = `audit.events.${type}`;
  const label = t(key);
  return label === key ? type : label;
}

async function loadAuditLogs() {
  loading.value = true;
  error.value = null;
  try {
    const response = await apiClient.auditLog(100);
    auditLogs.value = response;
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    error.value = t('audit.load_failed', { error: message });
  } finally {
    loading.value = false;
  }
}

function getLocationString(details: Record<string, unknown>): string {
  const parts: string[] = [];
  if (typeof details.city === 'string' && details.city.length > 0) parts.push(details.city);
  if (typeof details.country === 'string' && details.country.length > 0) parts.push(details.country);
  return parts.length > 0 ? parts.join(', ') : '—';
}

onMounted(() => {
  void loadAuditLogs();
});
</script>

<template>
  <AppLayout>
    <template #title>
      <h1 class="page-heading">{{ t('audit.heading') }}</h1>
      <p class="page-subtitle">{{ t('audit.subtitle') }}</p>
    </template>

    <section class="logs-view" data-test="logs-view">
      <article class="logs-panel panel">
        <div v-if="loading" class="logs-loading" data-test="logs-loading">
          {{ t('audit.loading') }}
        </div>
        <div v-else-if="error" class="logs-error" data-test="logs-error">{{ error }}</div>

        <div v-else class="logs-content">
          <div v-if="formattedAuditLogs.length === 0" class="logs-empty" data-test="logs-empty">
            {{ t('audit.empty') }}
          </div>
          <div v-else class="logs-table-wrap">
            <table class="logs-table">
              <thead>
                <tr>
                  <th>{{ t('audit.columns.time') }}</th>
                  <th>{{ t('audit.columns.event') }}</th>
                  <th>{{ t('audit.columns.user') }}</th>
                  <th>{{ t('audit.columns.ip_address') }}</th>
                  <th>{{ t('audit.columns.location') }}</th>
                  <th>{{ t('audit.columns.status') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="entry in formattedAuditLogs" :key="entry.id">
                  <td class="logs-table__time">{{ entry.formattedTime }}</td>
                  <td class="logs-table__event">{{ entry.eventLabel }}</td>
                  <td class="logs-table__user">{{ entry.user || '—' }}</td>
                  <td class="logs-table__ip">{{ entry.ip_address }}</td>
                  <td class="logs-table__location">{{ getLocationString(entry.details) }}</td>
                  <td>
                    <span class="logs-status" :class="entry.statusClass">
                      {{ entry.success ? t('audit.status.success') : t('audit.status.failure') }}
                    </span>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </article>
    </section>
  </AppLayout>
</template>

<style scoped>
.logs-view {
  max-width: 1400px;
  margin: 0 auto;
}

.logs-panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 12px;
  padding: 12px;
  box-shadow: var(--panel-shadow);
}

.logs-loading,
.logs-error,
.logs-empty {
  padding: 3rem;
  text-align: center;
  color: var(--text-muted);
}

.logs-error {
  color: var(--accent-red);
}

.logs-content {
  min-height: 320px;
}

.logs-table-wrap {
  overflow-x: auto;
}

.logs-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

.logs-table thead {
  background: var(--bg-card-soft);
  border-bottom: 1px solid var(--border-soft);
}

.logs-table th {
  padding: 14px 16px;
  text-align: left;
  font-weight: 600;
  color: var(--text-primary);
  font-size: 12px;
  white-space: nowrap;
}

.logs-table tbody tr {
  border-bottom: 1px solid var(--border-soft);
  transition: background 150ms ease;
}

.logs-table tbody tr:hover {
  background: var(--bg-card-soft);
}

.logs-table td {
  padding: 15px 16px;
  color: var(--text-secondary);
}

.logs-table__time {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}

.logs-table__event {
  font-weight: 600;
  color: var(--text-primary);
}

.logs-table__user {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}

.logs-table__ip {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}

.logs-table__location {
  color: var(--text-muted);
}

.logs-status {
  display: inline-block;
  padding: 4px 10px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
}

.logs-status.status-success {
  background: var(--accent-green-soft);
  color: var(--accent-green);
}

.logs-status.status-failure {
  background: var(--accent-red-soft);
  color: var(--accent-red);
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

@media (max-width: 820px) {
  .logs-panel {
    padding: 10px;
  }

  .logs-table th,
  .logs-table td {
    padding: 12px 10px;
  }
}
</style>
