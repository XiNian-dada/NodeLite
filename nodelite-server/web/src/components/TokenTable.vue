<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient, type SettingsAgentToken } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import { tokenRemaining, tokenSeverity } from '@/lib/format';
import DeleteAgentDialog from './DeleteAgentDialog.vue';
import TrafficControlStatus from './TrafficControlStatus.vue';

const props = defineProps<{
  agents: SettingsAgentToken[];
  authEnabled: boolean;
  twoFactorEnabled: boolean;
}>();
const emit = defineEmits<{ saved: []; deleted: [] }>();
const { t, locale } = useI18n();

interface ServiceDraft {
  serviceDate: string;
  serviceUnlimited: boolean;
  renewalPrice: string;
  trafficLimitBytes: number | null;
  trafficAccounting: SettingsAgentToken['traffic_accounting'];
  trafficThrottleKbps: number | null;
  saving: boolean;
  state: 'ok' | 'error' | null;
  message: string;
}

const drafts = reactive<Record<string, ServiceDraft>>({});
const deleteTarget = ref<SettingsAgentToken | null>(null);

function draftFromAgent(agent: SettingsAgentToken, existing?: ServiceDraft): ServiceDraft {
  return {
    serviceDate: dateInputValue(agent.service_expires_at),
    serviceUnlimited: agent.service_unlimited,
    renewalPrice: agent.renewal_price ?? '',
    trafficLimitBytes: agent.traffic_limit_bytes,
    trafficAccounting: agent.traffic_accounting,
    trafficThrottleKbps: agent.traffic_throttle_kbps,
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
      service_expires_at: draft.serviceUnlimited ? null : serviceExpiresAt(draft.serviceDate),
      service_unlimited: draft.serviceUnlimited,
      renewal_price: renewalPrice || null,
      traffic_limit_bytes: draft.trafficLimitBytes,
      traffic_accounting: draft.trafficAccounting,
      traffic_throttle_kbps: draft.trafficThrottleKbps,
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

function openDelete(agent: SettingsAgentToken): void {
  if (!props.authEnabled) return;
  deleteTarget.value = agent;
}

function closeDelete(): void {
  deleteTarget.value = null;
}

function agentDeleted(): void {
  deleteTarget.value = null;
  emit('deleted');
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
      agentToken: a,
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
          <th>{{ t('settings.tokens.expires_at') }}</th>
          <th>{{ t('settings.tokens.service_expires_at') }}</th>
          <th>{{ t('settings.tokens.renewal_price') }}</th>
          <th class="token-actions">{{ t('settings.tokens.actions') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.id" data-test="token-row">
          <td :data-label="t('settings.tokens.node')" class="node-cell">
            <div class="node-identity">
              <strong>{{ row.label }}</strong>
              <span class="subnote">{{ row.nodeId }} · Agent {{ row.agent }}</span>
              <span class="node-ip" :title="t('settings.tokens.ip')">{{ row.ip }}</span>
            </div>
          </td>
          <td :data-label="t('settings.tokens.status')">
            <span class="status-pill" :class="row.online ? 'online' : 'offline'">
              {{ row.status }}
            </span>
            <TrafficControlStatus
              :status="row.online ? (row.agentToken.traffic_control ?? null) : null"
              compact
            />
          </td>
          <td :data-label="t('settings.tokens.expires_at')">
            <div class="token-validity">
              <time :datetime="row.agentToken.token_expires_at ?? undefined">{{
                row.expiresAt
              }}</time>
              <span class="token-remaining" :class="row.severity">
                {{ t('settings.tokens.remaining') }} · {{ row.remaining }}
              </span>
            </div>
          </td>
          <td :data-label="t('settings.tokens.service_expires_at')">
            <input
              v-model="row.draft.serviceDate"
              class="meta-input"
              type="date"
              :disabled="row.draft.serviceUnlimited"
              :aria-label="`${row.label} · ${t('settings.tokens.service_expires_at')}`"
              data-test="service-expiry-input"
            />
            <label class="meta-check">
              <input
                v-model="row.draft.serviceUnlimited"
                type="checkbox"
                data-test="service-unlimited-input"
              />
              <span>{{ t('settings.tokens.service_unlimited_short') }}</span>
            </label>
          </td>
          <td :data-label="t('settings.tokens.renewal_price')">
            <input
              v-model="row.draft.renewalPrice"
              class="meta-input"
              type="text"
              maxlength="64"
              :aria-label="`${row.label} · ${t('settings.tokens.renewal_price')}`"
              :placeholder="t('settings.tokens.renewal_price_placeholder')"
              data-test="renewal-price-input"
            />
          </td>
          <td :data-label="t('settings.tokens.actions')" class="token-actions">
            <div class="row-actions">
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
              <button
                class="agent-delete"
                type="button"
                :disabled="!authEnabled"
                data-test="delete-agent"
                @click="openDelete(row.agentToken)"
              >
                {{ t('settings.tokens.delete') }}
              </button>
            </div>
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

    <DeleteAgentDialog
      v-if="deleteTarget"
      :agent="deleteTarget"
      :two-factor-enabled="twoFactorEnabled"
      @close="closeDelete"
      @deleted="agentDeleted"
    />
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 16px;
  container-type: inline-size;
  container-name: agent-table;
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
  table-layout: fixed;
}
.tokens th,
.tokens td {
  text-align: left;
  padding: 14px 12px;
  border-bottom: 1px solid var(--border-soft);
  vertical-align: top;
}
.tokens th {
  color: var(--text-muted);
  font-weight: 500;
  background: var(--bg-card-soft);
}
.tokens th:nth-child(1),
.tokens th:nth-child(2),
.tokens th:nth-child(3) {
  width: 20%;
}
.tokens th:nth-child(4) {
  width: 17%;
}
.tokens th:nth-child(5) {
  width: 13%;
}
.tokens .token-actions {
  text-align: right;
}
.row-actions {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
}
.node-identity,
.token-validity {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 7px;
  overflow-wrap: anywhere;
}
.node-identity strong {
  font-size: 14px;
  color: var(--text-primary);
}
.node-ip {
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  font-size: 12px;
}
.token-validity time {
  line-height: 1.6;
  font-variant-numeric: tabular-nums;
}
.token-remaining {
  display: inline-block;
  padding: 3px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg-card-soft);
  font-size: 12px;
  line-height: 1.6;
  font-variant-numeric: tabular-nums;
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
  min-width: 0;
  height: 36px;
  color: var(--text-primary);
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: var(--input-radius);
  padding: 0 9px;
  font: inherit;
  font-size: 12px;
}
.meta-input:focus {
  border-color: var(--accent-blue);
  box-shadow: 0 0 0 2px var(--accent-blue-soft);
  outline: none;
}
.meta-input:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
.meta-check {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--text-secondary);
  font-size: 12px;
  white-space: nowrap;
  margin-top: 10px;
}
.meta-check input {
  width: 14px;
  height: 14px;
  accent-color: var(--accent-green);
}
.meta-save {
  min-width: 64px;
  height: 32px;
  color: var(--text-primary);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: var(--btn-radius);
  font: inherit;
  font-size: 12px;
  font-weight: 550;
  cursor: pointer;
  transition:
    background-color 0.15s ease,
    border-color 0.15s ease,
    color 0.15s ease;
}
.meta-save:hover:not(:disabled) {
  background: var(--bg-elevated);
  border-color: var(--border-strong);
}
.meta-save:disabled {
  cursor: not-allowed;
  opacity: 0.58;
}
.agent-delete {
  min-width: 64px;
  height: 32px;
  color: var(--accent-red);
  background: transparent;
  border: 1px solid var(--border-soft);
  border-radius: var(--btn-radius);
  font: inherit;
  font-size: 12px;
  font-weight: 550;
  cursor: pointer;
  transition:
    background-color 0.15s ease,
    border-color 0.15s ease,
    color 0.15s ease;
}
.agent-delete:hover:not(:disabled) {
  background: var(--accent-red-soft);
  border-color: transparent;
}
.agent-delete:disabled {
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
@container agent-table (max-width: 1080px) {
  .tokens,
  .tokens tbody {
    display: block;
    border: 0;
    overflow: visible;
  }
  .tokens thead {
    display: none;
  }
  .tokens tr {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 18px 24px;
    border: 1px solid var(--border-soft);
    border-radius: var(--radius-md);
    padding: 18px;
    margin-bottom: 14px;
  }
  .tokens tr:last-child {
    margin-bottom: 0;
  }
  .tokens td {
    border-bottom: 0;
    display: block;
    min-width: 0;
    padding: 0;
    overflow-wrap: anywhere;
  }
  .tokens td::before {
    content: attr(data-label);
    display: block;
    margin-bottom: 8px;
    color: var(--text-muted);
    font-size: 12px;
  }
  .tokens .token-actions {
    text-align: left;
    grid-column: 1 / -1;
    border-top: 1px solid var(--border-soft);
    padding-top: 14px;
  }
  .row-actions {
    flex-direction: row;
    flex-wrap: wrap;
  }
  .row-actions button {
    min-width: 90px;
  }
  .meta-input {
    max-width: 260px;
  }
}
@container agent-table (max-width: 430px) {
  .tokens tr {
    grid-template-columns: minmax(0, 1fr);
    gap: 16px;
    padding: 14px;
  }
  .meta-input {
    max-width: none;
  }
}
</style>
