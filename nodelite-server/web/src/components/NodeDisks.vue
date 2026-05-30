<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { NodeStatus } from '@/api';
import { fmtBytes } from '@/lib/format';
import { uniqueDisks } from '@/lib/disks';

const props = defineProps<{ node: NodeStatus | null }>();

const { t } = useI18n();

const disks = computed(() =>
  uniqueDisks(props.node?.snapshot?.disks).map((d) => {
    const pct = d.used_percent ?? 0;
    return {
      device: d.device,
      mount_point: d.mount_point,
      fs_type: d.fs_type,
      pct,
      barWidth: Math.max(2, Math.min(100, pct)),
      severity: pct >= 90 ? 'bad' : pct >= 70 ? 'warn' : '',
      capacity: `${fmtBytes(d.used_bytes) ?? '—'} / ${fmtBytes(d.total_bytes) ?? '—'}`,
    };
  }),
);
</script>

<template>
  <div data-test="node-disks">
    <p v-if="disks.length === 0" class="placeholder" data-test="node-disks-empty">
      {{ t('node.no_disks') }}
    </p>
    <table v-else class="disks-table">
      <thead>
        <tr>
          <th>{{ t('node.disk.device') }}</th>
          <th>{{ t('node.disk.mount') }}</th>
          <th>{{ t('node.disk.filesystem') }}</th>
          <th>{{ t('node.disk.usage') }}</th>
          <th class="numeric">{{ t('node.disk.capacity') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(d, i) in disks" :key="i" data-test="disk-row">
          <td>{{ d.device }}</td>
          <td>{{ d.mount_point }}</td>
          <td>{{ d.fs_type }}</td>
          <td>
            <span class="disks-bar">
              <span :class="d.severity" :style="{ width: `${d.barWidth}%` }" />
            </span>
            {{ d.pct.toFixed(0) }}%
          </td>
          <td class="numeric">{{ d.capacity }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.placeholder {
  color: var(--text-muted);
  font-size: 13px;
  padding: 18px;
  margin: 0;
}
.disks-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
.disks-table th,
.disks-table td {
  text-align: left;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-soft);
}
.disks-table th {
  color: var(--text-muted);
  font-weight: 500;
}
.disks-table .numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.disks-bar {
  display: inline-block;
  width: 90px;
  height: 6px;
  border-radius: 999px;
  background: var(--bg-card-soft);
  overflow: hidden;
  margin-right: 8px;
  vertical-align: middle;
}
.disks-bar > span {
  display: block;
  height: 100%;
  background: var(--accent-green);
}
.disks-bar > span.warn {
  background: var(--accent-yellow);
}
.disks-bar > span.bad {
  background: var(--accent-red);
}
</style>
