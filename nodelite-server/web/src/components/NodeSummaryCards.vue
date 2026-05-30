<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { NodeStatus } from '@/api';
import { fmtBytes } from '@/lib/format';
import { totalDiskBytes, uniqueDisks, usedDiskBytes } from '@/lib/disks';

const props = defineProps<{ node: NodeStatus | null }>();

const { t } = useI18n();

const disk = computed(() => {
  const disks = uniqueDisks(props.node?.snapshot?.disks);
  const total = totalDiskBytes(disks);
  const used = usedDiskBytes(disks);
  const pct = total ? (used / total) * 100 : null;
  return {
    pctText: pct == null ? '—' : `${pct.toFixed(0)}%`,
    fillWidth: pct == null ? 0 : Math.max(2, Math.min(100, pct)),
    severity: pct == null ? '' : pct >= 90 ? 'bad' : pct >= 70 ? 'warn' : '',
    used: total ? (fmtBytes(used) ?? '—') : '—',
    total: total ? (fmtBytes(total) ?? '—') : '—',
  };
});

const load = computed(() => props.node?.snapshot?.load ?? null);
</script>

<template>
  <div class="summary-cards" data-test="node-summary-cards">
    <article class="panel small-card">
      <div class="label">{{ t('node.disk_usage') }}</div>
      <div class="value" data-test="summary-disk-pct">{{ disk.pctText }}</div>
      <div class="progress-bar">
        <div class="progress-fill" :class="disk.severity" :style="{ width: `${disk.fillWidth}%` }" />
      </div>
      <div class="sub">
        <span class="num">{{ disk.used }}</span><span>/</span><span class="num">{{ disk.total }}</span>
      </div>
    </article>

    <article class="panel small-card">
      <div class="label">{{ t('node.load') }}</div>
      <div class="value" data-test="summary-load">{{ load ? load.one.toFixed(2) : '—' }}</div>
      <div v-if="load" class="sub">
        <span class="num">{{ load.one.toFixed(2) }}</span>
        <span class="num">{{ load.five.toFixed(2) }}</span>
        <span class="num">{{ load.fifteen.toFixed(2) }}</span>
        <span>1/5/15m</span>
      </div>
    </article>
  </div>
</template>

<style scoped>
.summary-cards {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 14px;
}
.small-card {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 16px 18px;
}
.small-card .label {
  color: var(--text-muted);
  font-size: 13px;
}
.small-card .value {
  font-size: 26px;
  font-weight: 600;
  margin: 6px 0;
  letter-spacing: -0.02em;
}
.progress-bar {
  height: 6px;
  border-radius: 999px;
  background: var(--bg-card-soft);
  overflow: hidden;
  margin: 6px 0;
}
.progress-fill {
  height: 100%;
  background: var(--accent-green);
}
.progress-fill.warn {
  background: var(--accent-yellow);
}
.progress-fill.bad {
  background: var(--accent-red);
}
.small-card .sub {
  display: flex;
  gap: 8px;
  color: var(--text-muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.small-card .sub .num {
  color: var(--text-secondary);
}
</style>
