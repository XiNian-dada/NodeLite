<script setup lang="ts">
import { computed } from 'vue';
import { useNodesStore } from '@/stores/nodes';
import type { NodeListItem } from '@/api';

const nodesStore = useNodesStore();

type Tone = 'muted' | 'green' | 'yellow' | 'orange' | 'red';

interface MatrixRow {
  id: string;
  label: string;
  loadText: string;
  loadTone: Tone;
  cpuText: string;
  cpuTone: Tone;
  memoryText: string;
  memoryTone: Tone;
  status: 'online' | 'latency' | 'offline';
  statusLabelKey: string;
}

const PLACEHOLDER = '—';

const rows = computed(() =>
  [...nodesStore.nodes]
    .sort((a, b) => labelFor(a).localeCompare(labelFor(b)))
    .slice(0, 10)
    .map(toMatrixRow),
);

function labelFor(node: NodeListItem): string {
  return node.identity.node_label || node.identity.node_id;
}

function toMatrixRow(node: NodeListItem): MatrixRow {
  const cpu = node.snapshot?.cpu_usage_percent ?? null;
  const load = node.snapshot?.load.one ?? null;
  const memory = memoryPercent(node);
  const status = node.online
    ? node.latency_ms != null && node.latency_ms >= 200
      ? 'latency'
      : 'online'
    : 'offline';

  return {
    id: node.identity.node_id,
    label: labelFor(node),
    loadText: loadText(load),
    loadTone: loadTone(load),
    cpuText: percentText(cpu),
    cpuTone: usageTone(cpu),
    memoryText: percentText(memory),
    memoryTone: usageTone(memory),
    status,
    statusLabelKey:
      status === 'offline'
        ? 'common.offline'
        : status === 'latency'
          ? 'common.latency_warn'
          : 'common.online',
  };
}

function memoryPercent(node: NodeListItem): number | null {
  const memory = node.snapshot?.memory;
  if (!memory) return null;
  return (memory.used_bytes / Math.max(memory.total_bytes, 1)) * 100;
}

function percentText(value: number | null): string {
  return value == null ? PLACEHOLDER : `${value.toFixed(0)}%`;
}

function loadText(value: number | null): string {
  return value == null ? PLACEHOLDER : value.toFixed(2);
}

function loadTone(value: number | null): Tone {
  if (value == null) return 'muted';
  if (value < 1) return 'green';
  if (value < 2) return 'yellow';
  if (value < 4) return 'orange';
  return 'red';
}

function usageTone(value: number | null): Tone {
  if (value == null) return 'muted';
  if (value < 40) return 'green';
  if (value < 70) return 'yellow';
  if (value < 85) return 'orange';
  return 'red';
}
</script>

<template>
  <article class="panel health-matrix" data-test="node-health-matrix">
    <div class="panel-head">
      <div class="panel-title">
        <span>{{ $t('index.matrix.title') }}</span>
        <small>{{ $t('index.matrix.subtitle') }}</small>
      </div>
      <button type="button" class="panel-action">
        {{ $t('index.matrix.more') }}
      </button>
    </div>

    <div v-if="rows.length === 0" class="empty" data-test="health-matrix-empty">
      {{ $t('index.matrix.empty') }}
    </div>
    <table v-else class="matrix-table">
      <thead>
        <tr>
          <th class="row-head">{{ $t('index.matrix.col_node') }}</th>
          <th>{{ $t('index.matrix.col_current_load') }}</th>
          <th>{{ $t('index.node.cpu') }}</th>
          <th>{{ $t('index.node.memory') }}</th>
          <th>{{ $t('index.matrix.col_status') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.id" data-test="health-matrix-row">
          <td class="row-head">{{ row.label }}</td>
          <td>
            <div class="matrix-cell" :class="row.loadTone" data-test="health-matrix-load">
              {{ row.loadText }}
            </div>
          </td>
          <td>
            <div class="matrix-cell" :class="row.cpuTone" data-test="health-matrix-cpu">
              {{ row.cpuText }}
            </div>
          </td>
          <td>
            <div class="matrix-cell" :class="row.memoryTone" data-test="health-matrix-memory">
              {{ row.memoryText }}
            </div>
          </td>
          <td>
            <div class="status-cell" :class="row.status" data-test="health-matrix-status">
              <span class="status-dot" />
              <span>{{ $t(row.statusLabelKey) }}</span>
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
  box-shadow: var(--panel-shadow);
  padding: 18px 20px;
  min-height: 100%;
}
.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
}
.panel-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: var(--text-secondary);
  font-size: 14px;
  font-weight: 600;
}
.panel-title small {
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 400;
}
.panel-action {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  border: 0;
  background: transparent;
  color: var(--accent-blue);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
  padding: 0;
}
.panel-action:hover {
  color: var(--text-secondary);
}
.empty {
  color: var(--text-muted);
  font-size: 13px;
  padding: 10px 0 2px;
}
.matrix-table {
  width: 100%;
  border-collapse: separate;
  border-spacing: 0 8px;
  table-layout: fixed;
  font-size: 12px;
}
.matrix-table th {
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 500;
  padding: 8px 4px;
  text-align: center;
}
.matrix-table th.row-head {
  width: 24%;
  padding-left: 4px;
  text-align: left;
}
.matrix-table td {
  padding: 4px;
}
.matrix-table td.row-head {
  overflow: hidden;
  padding-left: 0;
  color: var(--text-secondary);
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.matrix-cell {
  min-width: 0;
  border-radius: 6px;
  border: 1px solid transparent;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
  font-weight: 500;
  padding: 4px;
  text-align: center;
}
.matrix-cell.muted {
  background: var(--bg-card-soft);
  color: var(--text-dim);
}
.matrix-cell.green {
  background: var(--accent-green-soft);
  border-color: rgba(37, 228, 135, 0.16);
  color: var(--accent-green);
}
.matrix-cell.yellow {
  background: var(--accent-yellow-soft);
  border-color: rgba(245, 197, 66, 0.18);
  color: var(--accent-yellow);
}
.matrix-cell.orange {
  background: rgba(249, 115, 22, 0.16);
  border-color: rgba(249, 115, 22, 0.18);
  color: #ff9d42;
}
.matrix-cell.red {
  background: var(--accent-red-soft);
  border-color: rgba(255, 77, 109, 0.18);
  color: var(--accent-red);
}
.status-cell {
  align-items: center;
  display: inline-flex;
  font-size: 12px;
  gap: 7px;
  justify-content: center;
  min-width: 76px;
  padding: 4px 8px;
  border: 1px solid transparent;
  border-radius: 999px;
  white-space: nowrap;
}
.status-dot {
  border-radius: 50%;
  height: 6px;
  width: 6px;
}
.status-cell.online {
  background: color-mix(in srgb, var(--accent-green) 10%, transparent);
  border-color: color-mix(in srgb, var(--accent-green) 22%, transparent);
  color: var(--accent-green);
}
.status-cell.online .status-dot {
  background: var(--accent-green);
}
.status-cell.latency {
  background: color-mix(in srgb, var(--accent-yellow) 12%, transparent);
  border-color: color-mix(in srgb, var(--accent-yellow) 24%, transparent);
  color: var(--accent-yellow);
}
.status-cell.latency .status-dot {
  background: var(--accent-yellow);
}
.status-cell.offline {
  background: color-mix(in srgb, var(--accent-red) 10%, transparent);
  border-color: color-mix(in srgb, var(--accent-red) 22%, transparent);
  color: var(--accent-red);
}
.status-cell.offline .status-dot {
  background: var(--accent-red);
}
@media (max-width: 480px) {
  .panel {
    padding: 16px;
  }
  .panel-head {
    align-items: flex-start;
  }
  .panel-title {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
  }
}
</style>
