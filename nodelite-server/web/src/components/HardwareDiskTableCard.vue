<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import type { HardwareDiskRow } from '@/lib/nodeHardwareModel';

defineProps<{ rows: HardwareDiskRow[]; diskCount: number }>();

const { t } = useI18n();
</script>

<template>
  <article class="hardware-card disk-table-card" data-test="node-disks">
    <header class="card-head card-head--row">
      <div>
        <span class="card-kicker">{{ t('node.hardware.partitions') }}</span>
        <strong>{{ t('node.mounted_disks') }}</strong>
      </div>
      <span class="card-count">{{ t('node.hardware.partition_count', { count: diskCount }) }}</span>
    </header>
    <p v-if="rows.length === 0" class="placeholder" data-test="node-disks-empty">
      {{ t('node.no_disks') }}
    </p>
    <div v-else class="disk-table">
      <div class="disk-head">
        <span>{{ t('node.disk.device') }}</span>
        <span>{{ t('node.disk.mount') }}</span>
        <span>{{ t('node.disk.filesystem') }}</span>
        <span>{{ t('node.disk.usage') }}</span>
        <span>{{ t('node.disk.capacity') }}</span>
      </div>
      <div
        v-for="row in rows"
        :key="`${row.device}:${row.mount}`"
        class="disk-row"
        data-test="disk-row"
      >
        <span class="device" :data-label="t('node.disk.device')">{{ row.device }}</span>
        <span :data-label="t('node.disk.mount')">{{ row.mount }}</span>
        <span :data-label="t('node.disk.filesystem')"
          ><em>{{ row.fs }}</em></span
        >
        <span class="usage-cell" :data-label="t('node.disk.usage')">
          <span class="usage-track">
            <span :class="row.severity" :style="{ width: `${row.bar}%` }" />
          </span>
          <span class="usage-value">{{ Math.round(row.pct) }}%</span>
        </span>
        <span class="capacity" :data-label="t('node.disk.capacity')">{{ row.capacity }}</span>
      </div>
    </div>
  </article>
</template>

<style scoped>
.hardware-card {
  min-width: 0;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card);
  padding: 16px;
}

.card-head {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 14px;
}

.card-head--row {
  flex-direction: row;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.card-head strong {
  color: var(--text-primary);
  font-size: 15px;
  font-weight: 600;
}

.card-kicker,
.card-count,
.placeholder {
  color: var(--text-muted);
  font-size: 12px;
}

.disk-table {
  overflow-x: auto;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
}

.disk-head,
.disk-row {
  display: grid;
  grid-template-columns: minmax(140px, 1.1fr) minmax(80px, 0.7fr) minmax(84px, 0.6fr) minmax(
      160px,
      1fr
    ) minmax(120px, 0.8fr);
  gap: 12px;
  align-items: center;
  min-width: 720px;
  padding: 11px 12px;
}

.disk-head {
  color: var(--text-muted);
  background: var(--bg-card-soft);
  font-size: 12px;
}

.disk-row {
  border-top: 1px solid var(--border-soft);
  color: var(--text-secondary);
  font-size: 13px;
}

.device,
.capacity {
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

.disk-row em {
  display: inline-flex;
  border-radius: 6px;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  padding: 3px 8px;
  font-style: normal;
}

.usage-cell {
  display: grid;
  grid-template-columns: minmax(80px, 1fr) auto;
  gap: 10px;
  align-items: center;
  font-variant-numeric: tabular-nums;
}

.usage-track {
  height: 6px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--bg-card-soft);
}

.usage-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: currentColor;
}

.usage-track .ok {
  background: var(--accent-green);
}

.usage-track .warn {
  background: var(--accent-yellow);
}

.usage-track .bad {
  background: var(--accent-red);
}

@media (max-width: 560px) {
  .hardware-card {
    padding: 14px;
  }

  .card-head--row {
    flex-direction: column;
  }

  .disk-table {
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow: visible;
    border: 0;
  }

  .disk-head {
    display: none;
  }

  .disk-row {
    grid-template-columns: minmax(0, 1fr);
    gap: 8px;
    min-width: 0;
    border: 1px solid var(--border-soft);
    border-radius: 8px;
    background: var(--bg-card-soft);
    padding: 12px;
  }

  .disk-row > span {
    display: grid;
    grid-template-columns: minmax(82px, 0.42fr) minmax(0, 1fr);
    gap: 10px;
    align-items: center;
    overflow-wrap: anywhere;
  }

  .disk-row > span::before {
    content: attr(data-label);
    color: var(--text-muted);
    font-size: 12px;
  }

  .usage-cell {
    grid-template-columns: minmax(82px, 0.42fr) minmax(0, 1fr) auto;
  }

  .usage-value {
    font-variant-numeric: tabular-nums;
  }
}
</style>
