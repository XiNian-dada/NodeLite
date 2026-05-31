<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { SettingsAgentToken } from '@/api';
import { tokenRemaining, tokenSeverity } from '@/lib/format';

const props = defineProps<{ agents: SettingsAgentToken[] }>();
const { t, locale } = useI18n();

function fmtDateTime(value: string | null): string {
  if (!value) return t('settings.token.no_expiry');
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? new Date(ms).toLocaleString(locale.value) : value;
}

function remainingText(seconds: number | null): string {
  const r = tokenRemaining(seconds);
  switch (r.kind) {
    case 'none':
      return t('settings.token.no_expiry');
    case 'expired':
      return t('settings.token.expired');
    case 'days_hours':
      return t('settings.duration.days_hours', { days: r.days, hours: r.hours });
    case 'minutes':
      return t('settings.duration.minutes', { minutes: r.minutes });
  }
}

const rows = computed(() =>
  props.agents.map((a) => ({
    id: a.node_id,
    label: a.node_label || a.node_id,
    nodeId: a.node_id,
    status: a.online ? t('common.online') : t('common.offline'),
    online: a.online,
    agent: a.agent_version ?? t('common.not_available'),
    ip: a.remote_ip ?? t('common.not_available'),
    expiresAt: fmtDateTime(a.token_expires_at),
    remaining: remainingText(a.token_expires_in_secs),
    severity: tokenSeverity(a.token_expires_in_secs),
  })),
);
</script>

<template>
  <article class="panel" data-test="token-table">
    <h2 class="card-title">{{ t('settings.tokens.title') }}</h2>
    <p v-if="rows.length === 0" class="empty" data-test="token-table-empty">
      {{ t('settings.tokens.empty') }}
    </p>
    <table v-else class="tokens">
      <thead>
        <tr>
          <th>{{ t('settings.tokens.node') }}</th>
          <th>{{ t('settings.tokens.status') }}</th>
          <th>{{ t('settings.tokens.agent') }}</th>
          <th>{{ t('settings.tokens.ip') }}</th>
          <th>{{ t('settings.tokens.expires_at') }}</th>
          <th class="numeric">{{ t('settings.tokens.remaining') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.id" data-test="token-row">
          <td>
            {{ row.label }}<div class="subnote">{{ row.nodeId }}</div>
          </td>
          <td>{{ row.status }}</td>
          <td>{{ row.agent }}</td>
          <td>{{ row.ip }}</td>
          <td>{{ row.expiresAt }}</td>
          <td class="numeric" :class="row.severity">{{ row.remaining }}</td>
        </tr>
      </tbody>
    </table>
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 18px 20px;
}
.card-title {
  margin: 0 0 12px;
  font-size: 14px;
  font-weight: 600;
}
.empty {
  color: var(--text-muted);
  font-size: 13px;
  margin: 0;
}
.tokens {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
.tokens th,
.tokens td {
  text-align: left;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-soft);
  vertical-align: top;
}
.tokens th {
  color: var(--text-muted);
  font-weight: 500;
}
.tokens .numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.subnote {
  color: var(--text-muted);
  font-size: 11px;
}
.expired {
  color: var(--accent-red);
}
.expiring {
  color: var(--accent-yellow);
}
.ok {
  color: var(--accent-green);
}
</style>
