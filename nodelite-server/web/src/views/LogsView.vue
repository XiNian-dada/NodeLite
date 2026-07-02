<script setup lang="ts">
import { ref, onMounted, computed } from 'vue';
import AppLayout from '@/components/AppLayout.vue';
import { apiClient } from '@/api';
import type { AuditLogEntry } from '@/api';

type TabName = 'agent' | 'server' | 'audit';
const activeTab = ref<TabName>('audit');

const auditLogs = ref<AuditLogEntry[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

const formattedAuditLogs = computed(() => {
  return auditLogs.value.map((entry) => ({
    ...entry,
    formattedTime: new Date(entry.timestamp).toLocaleString(),
    eventLabel: formatEventType(entry.event_type),
    statusClass: entry.success ? 'status-success' : 'status-failure',
  }));
});

function formatEventType(type: string): string {
  const labels: Record<string, string> = {
    login_success: 'Login Success',
    login_failure: 'Login Failure',
    totp_verify_success: '2FA Verified',
    totp_verify_failure: '2FA Failed',
    node_connected: 'Node Connected',
    token_invalid: 'Invalid Token',
    rate_limit_exceeded: 'Rate Limited',
  };
  return labels[type] || type;
}

async function loadAuditLogs() {
  loading.value = true;
  error.value = null;
  try {
    const response = await apiClient.auditLog(100);
    auditLogs.value = response;
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'Failed to load audit logs';
  } finally {
    loading.value = false;
  }
}

function getLocationString(details: Record<string, any>): string {
  const parts = [];
  if (details.city) parts.push(details.city);
  if (details.country) parts.push(details.country);
  return parts.length > 0 ? parts.join(', ') : '—';
}

onMounted(() => {
  void loadAuditLogs();
});
</script>

<template>
  <AppLayout>
    <template #title>
      <h1 class="page-heading">Logs</h1>
      <p class="page-subtitle">View system and audit logs</p>
    </template>

    <section class="logs-view" data-test="logs-view">
      <nav class="logs-tabs">
        <button
          class="logs-tab"
          :class="{ 'logs-tab--active': activeTab === 'audit' }"
          @click="activeTab = 'audit'"
        >
          Audit Log
        </button>
        <button
          class="logs-tab"
          :class="{ 'logs-tab--active': activeTab === 'agent' }"
          @click="activeTab = 'agent'"
        >
          Agent Logs
        </button>
        <button
          class="logs-tab"
          :class="{ 'logs-tab--active': activeTab === 'server' }"
          @click="activeTab = 'server'"
        >
          Server Logs
        </button>
      </nav>

      <article class="logs-panel panel">
        <div v-if="loading" class="logs-loading">Loading...</div>
        <div v-else-if="error" class="logs-error">{{ error }}</div>

        <div v-else-if="activeTab === 'audit'" class="logs-content">
          <div v-if="formattedAuditLogs.length === 0" class="logs-empty">
            No audit log entries found.
          </div>
          <div v-else class="logs-table-wrap">
            <table class="logs-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Event</th>
                  <th>User</th>
                  <th>IP Address</th>
                  <th>Location</th>
                  <th>Status</th>
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
                      {{ entry.success ? 'Success' : 'Failure' }}
                    </span>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <div v-else-if="activeTab === 'agent'" class="logs-content">
          <div class="logs-placeholder">Agent logs coming soon...</div>
        </div>

        <div v-else-if="activeTab === 'server'" class="logs-content">
          <div class="logs-placeholder">Server logs coming soon...</div>
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

.logs-tabs {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
  border-bottom: 2px solid #E7E1D7;
}

.logs-tab {
  padding: 0.75rem 1.5rem;
  background: none;
  border: none;
  border-bottom: 3px solid transparent;
  color: #5C635D;
  font-size: 0.95rem;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  margin-bottom: -2px;
}

.logs-tab:hover {
  color: #1F2421;
  background: #FBF9F5;
}

.logs-tab--active {
  color: #C4612F;
  border-bottom-color: #C4612F;
}

.logs-panel {
  background: #FFFFFF;
  border-radius: 12px;
  padding: 1.5rem;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
  border: 1px solid #E7E1D7;
}

.logs-loading,
.logs-error,
.logs-empty,
.logs-placeholder {
  padding: 3rem;
  text-align: center;
  color: #5C635D;
}

.logs-error {
  color: #C4612F;
}

.logs-content {
  min-height: 400px;
}

.logs-table-wrap {
  overflow-x: auto;
}

.logs-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.9rem;
}

.logs-table thead {
  background: #FBF9F5;
  border-bottom: 2px solid #E7E1D7;
}

.logs-table th {
  padding: 0.75rem 1rem;
  text-align: left;
  font-weight: 600;
  color: #1F2421;
  font-size: 0.85rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.logs-table tbody tr {
  border-bottom: 1px solid #E7E1D7;
  transition: background 0.15s ease;
}

.logs-table tbody tr:hover {
  background: #FBF9F5;
}

.logs-table td {
  padding: 1rem;
  color: #1F2421;
}

.logs-table__time {
  font-family: 'Courier New', monospace;
  font-size: 0.85rem;
  color: #5C635D;
}

.logs-table__event {
  font-weight: 500;
}

.logs-table__user {
  font-family: 'Courier New', monospace;
  font-size: 0.9rem;
}

.logs-table__ip {
  font-family: 'Courier New', monospace;
  font-size: 0.85rem;
  color: #5C635D;
}

.logs-table__location {
  color: #5C635D;
}

.logs-status {
  display: inline-block;
  padding: 0.25rem 0.75rem;
  border-radius: 999px;
  font-size: 0.8rem;
  font-weight: 500;
}

.logs-status.status-success {
  background: #F2E3D6;
  color: #A94E22;
}

.logs-status.status-failure {
  background: #FBF9F5;
  color: #5C635D;
}
</style>
