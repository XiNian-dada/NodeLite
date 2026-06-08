<script setup lang="ts">
import { computed, reactive, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient, type SettingsAgentToken } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import { tokenRemaining, tokenSeverity } from '@/lib/format';

const props = defineProps<{ agents: SettingsAgentToken[] }>();
const emit = defineEmits<{ saved: [] }>();
const { t, locale } = useI18n();

interface ServiceDraft {
  serviceDate: string;
  renewalPrice: string;
  saving: boolean;
  state: 'ok' | 'error' | null;
  message: string;
}

const drafts = reactive<Record<string, ServiceDraft>>({});

function draftFromAgent(agent: SettingsAgentToken, existing?: ServiceDraft): ServiceDraft {
  return {
    serviceDate: dateInputValue(agent.service_expires_at),
    renewalPrice: agent.renewal_price ?? '',
    saving: false,
    state: existing?.state ?? null,
    message: existing?.message ?? '',
  };
}

function fmtDateTime(value: string | null): string {
  if (!value) return t('settings.token.no_expiry');
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? new Date(ms).toLocaleString(locale.value) : value;
}

function dateInputValue(value: string | null): string {
  if (!value) return '';
  const ms = Date.parse(value);
  if (Number.isFinite(ms)) return new Date(ms).toISOString().slice(0, 10);
  return /^\d{4}-\d{2}-\d{2}/.test(value) ? value.slice(0, 10) : '';
}

function serviceExpiresAt(value: string): string | null {
  return value ? `${value}T00:00:00Z` : null;
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

watch(
  () => props.agents,
  (agents) => {
    const ids = new Set(agents.map((agent) => agent.node_id));
    for (const id of Object.keys(drafts)) {
      if (!ids.has(id)) delete drafts[id];
    }
    for (const agent of agents) {
      const existing = drafts[agent.node_id];
      drafts[agent.node_id] = draftFromAgent(agent, existing);
    }
  },
  { immediate: true, flush: 'sync' },
);

async function saveServiceMetadata(nodeId: string): Promise<void> {
  const draft = drafts[nodeId];
  if (!draft) return;
  draft.saving = true;
  draft.state = null;
  draft.message = '';
  try {
    const renewalPrice = draft.renewalPrice.trim();
    const resp = await apiClient.updateNodeServiceMetadata(nodeId, {
      service_expires_at: serviceExpiresAt(draft.serviceDate),
      renewal_price: renewalPrice || null,
    });
    draft.renewalPrice = renewalPrice;
    draft.state = 'ok';
    draft.message = resp.message || t('settings.tokens.service_meta_saved');
    emit('saved');
  } catch (e) {
    if (e instanceof ApiAbortError) return;
    draft.state = 'error';
    draft.message = t('settings.tokens.service_meta_failed', {
      error: messageFromError(e, 'unknown'),
    });
  } finally {
    draft.saving = false;
  }
}

const rows = computed(() =>
  props.agents.map((a) => {
    const draft = drafts[a.node_id] ?? draftFromAgent(a);
    return {
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
      draft,
    };
  }),
);
</script>

<template>
  <article class="panel" data-test="token-table">
    <header class="card-head">
      <div>
        <span class="card-kicker">{{ t('settings.summary.token_health') }}</span>
        <h2 class="card-title">{{ t('settings.tokens.title') }}</h2>
      </div>
      <strong class="agent-count">{{ agents.length }}</strong>
    </header>
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
          <th>{{ t('settings.tokens.service_expires_at') }}</th>
          <th>{{ t('settings.tokens.renewal_price') }}</th>
          <th class="actions">{{ t('settings.tokens.actions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.id" data-test="token-row">
          <td :data-label="t('settings.tokens.node')">
            {{ row.label }}<div class="subnote">{{ row.nodeId }}</div>
          </td>
          <td :data-label="t('settings.tokens.status')">
            <span class="status-pill" :class="row.online ? 'online' : 'offline'">
              {{ row.status }}
            </span>
          </td>
          <td :data-label="t('settings.tokens.agent')">{{ row.agent }}</td>
          <td :data-label="t('settings.tokens.ip')">{{ row.ip }}</td>
          <td :data-label="t('settings.tokens.expires_at')">{{ row.expiresAt }}</td>
          <td :data-label="t('settings.tokens.remaining')" class="numeric" :class="row.severity">
            {{ row.remaining }}
          </td>
          <td :data-label="t('settings.tokens.service_expires_at')">
            <input
              v-model="row.draft.serviceDate"
              class="meta-input"
              type="date"
              data-test="service-expiry-input"
            />
          </td>
          <td :data-label="t('settings.tokens.renewal_price')">
            <input
              v-model="row.draft.renewalPrice"
              class="meta-input"
              type="text"
              maxlength="64"
              :placeholder="t('settings.tokens.renewal_price_placeholder')"
              data-test="renewal-price-input"
            />
          </td>
          <td :data-label="t('settings.tokens.actions')" class="actions">
            <button
              class="meta-save"
              type="button"
              :disabled="row.draft.saving"
              data-test="service-meta-save"
              @click="saveServiceMetadata(row.nodeId)"
            >
              {{
                row.draft.saving
                  ? t('settings.tokens.service_meta_saving')
                  : t('settings.tokens.service_meta_save')
              }}
            </button>
            <div
              v-if="row.draft.message"
              class="meta-message"
              :class="row.draft.state"
              data-test="service-meta-message"
            >
              {{ row.draft.message }}
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 16px;
}
.card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 14px;
}
.card-kicker {
  display: block;
  color: var(--text-muted);
  font-size: 12px;
  margin-bottom: 4px;
}
.card-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.agent-count {
  color: var(--text-primary);
  font-size: 22px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.empty {
  color: var(--text-muted);
  font-size: 13px;
  margin: 0;
}
.tokens {
  width: 100%;
  overflow: hidden;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
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
  background: var(--bg-card-soft);
}
.tokens .numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.tokens .actions {
  text-align: right;
}
.subnote {
  color: var(--text-muted);
  font-size: 11px;
}
.status-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
}
.status-pill::before {
  content: '';
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
}
.status-pill.online {
  color: var(--accent-green);
}
.status-pill.offline {
  color: var(--chart-network-up);
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
.meta-input {
  width: 100%;
  min-width: 112px;
  height: 32px;
  color: var(--text-primary);
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 7px;
  padding: 0 9px;
  font: inherit;
  font-size: 12px;
}
.meta-input:focus {
  border-color: var(--border-strong);
  outline: none;
}
.meta-save {
  min-width: 64px;
  height: 32px;
  color: var(--text-primary);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: 7px;
  font: inherit;
  font-size: 12px;
  font-weight: 600;
}
.meta-save:disabled {
  cursor: not-allowed;
  opacity: 0.58;
}
.meta-message {
  margin-top: 6px;
  font-size: 11px;
}
.meta-message.error {
  color: var(--accent-red);
}
.meta-message.ok {
  color: var(--accent-green);
}
@media (max-width: 640px) {
  .tokens,
  .tokens thead,
  .tokens tbody,
  .tokens tr,
  .tokens th,
  .tokens td {
    display: block;
  }
  .tokens thead {
    display: none;
  }
  .tokens tr {
    border-bottom: 1px solid var(--border-soft);
    padding: 10px 0;
  }
  .tokens tr:last-child {
    border-bottom: 0;
  }
  .tokens td {
    border-bottom: 0;
    display: grid;
    grid-template-columns: minmax(86px, 0.42fr) minmax(0, 1fr);
    gap: 10px;
    padding: 5px 0;
    overflow-wrap: anywhere;
  }
  .tokens td::before {
    content: attr(data-label);
    color: var(--text-muted);
  }
  .tokens .numeric {
    text-align: left;
  }
  .tokens .actions {
    text-align: left;
  }
  .meta-input {
    max-width: 220px;
  }
}
</style>
