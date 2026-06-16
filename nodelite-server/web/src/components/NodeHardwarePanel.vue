<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { NodeStatus } from '@/api';
import { buildNodeHardwareModel } from '@/lib/nodeHardwareModel';

const props = defineProps<{ node: NodeStatus | null }>();

const { t } = useI18n();

const model = computed(() => buildNodeHardwareModel(props.node, t));
</script>

<template>
  <div class="hardware-panel" data-test="node-hardware-panel">
    <section class="hardware-grid hardware-grid--top">
      <article class="hardware-card spec-card" data-test="hardware-spec-card">
        <header class="card-head">
          <span class="card-kicker">{{ t('node.hardware.system') }}</span>
          <strong>{{ t('node.info.title') }}</strong>
        </header>
        <div class="spec-rows">
          <template v-for="row in model.specRows" :key="row.label">
            <span class="spec-label">{{ row.label }}</span>
            <strong class="spec-value">{{ row.value }}</strong>
          </template>
        </div>
      </article>

      <article class="hardware-card storage-card" data-test="hardware-storage-card">
        <header class="card-head">
          <span class="card-kicker">{{ t('node.hardware.storage') }}</span>
          <strong>{{ t('node.disk_usage') }}</strong>
        </header>
        <div class="storage-body">
          <div
            class="donut"
            :style="{ '--pct': `${model.diskPercentBar}%` }"
            :aria-label="model.diskPercentText"
          >
            <div class="donut__content">
              <strong>{{ model.diskPercentText }}</strong>
              <span>{{ t('node.hardware.used') }}</span>
            </div>
          </div>
          <div class="storage-stats">
            <span>{{ t('node.hardware.total') }}</span>
            <strong>{{ model.totalDiskText }}</strong>
            <span>{{ t('node.hardware.used') }}</span>
            <strong>{{ model.usedDiskText }}</strong>
            <span>{{ t('node.hardware.available') }}</span>
            <strong>{{ model.availableDiskText }}</strong>
          </div>
        </div>
      </article>

      <article class="hardware-card filesystem-card" data-test="hardware-filesystem-card">
        <header class="card-head">
          <span class="card-kicker">{{ t('node.hardware.filesystems') }}</span>
          <strong>{{ t('node.disk.filesystem') }}</strong>
        </header>
        <div v-if="model.filesystemRows.length" class="filesystem-list">
          <div v-for="row in model.filesystemRows" :key="row.name" class="filesystem-row">
            <span class="dot" />
            <strong>{{ row.name }}</strong>
            <span>{{ row.total }}</span>
            <span>{{ Math.round(row.pct) }}%</span>
          </div>
        </div>
        <p v-else class="placeholder">{{ t('node.no_disks') }}</p>
        <div class="partition-count">
          <span>{{ t('node.hardware.partitions') }}</span>
          <strong>{{ model.disks.length }}</strong>
        </div>
      </article>
    </section>

    <section class="summary-strip" data-test="hardware-summary-cards">
      <article
        v-for="card in model.summaryCards"
        :key="card.key"
        class="summary-card"
        :class="`summary-card--${card.tone}`"
      >
        <span class="summary-label">{{ card.label }}</span>
        <strong>{{ card.value }}</strong>
        <div v-if="card.bar != null" class="summary-bar">
          <span :style="{ width: `${card.bar}%` }" />
        </div>
        <small>{{ card.sub }}</small>
      </article>
    </section>

    <section class="hardware-grid hardware-grid--bottom">
      <article class="hardware-card disk-table-card" data-test="node-disks">
        <header class="card-head card-head--row">
          <div>
            <span class="card-kicker">{{ t('node.hardware.partitions') }}</span>
            <strong>{{ t('node.mounted_disks') }}</strong>
          </div>
          <span class="card-count">{{
            t('node.hardware.partition_count', { count: model.disks.length })
          }}</span>
        </header>
        <p v-if="model.diskRows.length === 0" class="placeholder" data-test="node-disks-empty">
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
            v-for="row in model.diskRows"
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

      <article class="hardware-card health-card" data-test="hardware-health-card">
        <header class="card-head">
          <span class="card-kicker">{{ t('node.hardware.health.title') }}</span>
          <strong>{{ t('node.hardware.health.summary') }}</strong>
        </header>
        <div class="health-list">
          <div v-for="row in model.healthRows" :key="row.label" class="health-row">
            <span>{{ row.label }}</span>
            <strong :class="row.state">{{ row.value }}</strong>
          </div>
        </div>
      </article>
    </section>
  </div>
</template>

<style scoped>
.hardware-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.hardware-grid {
  display: grid;
  gap: 14px;
}

.hardware-grid--top {
  grid-template-columns: minmax(0, 1.05fr) minmax(0, 1fr) minmax(0, 0.95fr);
}

.hardware-grid--bottom {
  grid-template-columns: minmax(0, 1.8fr) minmax(260px, 0.75fr);
  align-items: stretch;
}

.hardware-card,
.summary-card {
  min-width: 0;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card);
}

.hardware-card {
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
.placeholder,
.summary-label,
.summary-card small,
.spec-label,
.storage-stats span,
.partition-count span {
  color: var(--text-muted);
  font-size: 12px;
}

.spec-rows {
  display: grid;
  grid-template-columns: minmax(86px, auto) minmax(0, 1fr);
  gap: 12px 18px;
}

.spec-value {
  min-width: 0;
  color: var(--text-primary);
  font-size: 13px;
  font-weight: 500;
  text-align: right;
  overflow-wrap: anywhere;
}

.storage-body {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  gap: 18px;
  align-items: center;
}

.donut {
  --pct: 0%;
  position: relative;
  display: grid;
  place-items: center;
  width: 144px;
  aspect-ratio: 1;
  border-radius: 50%;
  background: conic-gradient(var(--text-primary) var(--pct), var(--bg-card-soft) 0);
}

.donut::after {
  content: '';
  position: absolute;
  inset: 16px;
  border-radius: inherit;
  background: var(--bg-card);
}

.donut__content {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.donut strong {
  color: var(--text-primary);
  font-size: 32px;
  line-height: 1;
}

.donut span {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1;
}

.storage-stats {
  display: grid;
  grid-template-columns: minmax(72px, auto) minmax(0, 1fr);
  gap: 8px 12px;
  align-items: baseline;
}

.storage-stats strong,
.partition-count strong {
  color: var(--text-primary);
  font-size: 20px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.filesystem-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.filesystem-row {
  display: grid;
  grid-template-columns: auto 1fr auto auto;
  gap: 10px;
  align-items: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.filesystem-row strong {
  color: var(--text-primary);
}

.dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--text-secondary);
}

.partition-count {
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-top: 1px solid var(--border-soft);
  margin-top: 16px;
  padding-top: 14px;
}

.summary-strip {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 14px;
}

.summary-card {
  min-height: 118px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 8px;
  padding: 14px;
}

.summary-card strong {
  color: var(--text-primary);
  font-size: 18px;
  font-weight: 600;
  line-height: 1.2;
  overflow-wrap: anywhere;
}

.summary-card small {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.summary-bar,
.usage-track {
  overflow: hidden;
  border-radius: 999px;
  background: var(--bg-card-soft);
}

.summary-bar {
  height: 5px;
}

.summary-bar span,
.usage-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: currentColor;
}

.summary-card--blue {
  color: var(--chart-cpu);
}

.summary-card--green {
  color: var(--chart-memory);
}

.summary-card--red {
  color: var(--accent-red);
}

.summary-card--neutral {
  color: var(--text-secondary);
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

.health-list {
  display: flex;
  flex-direction: column;
}

.health-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  border-bottom: 1px solid var(--border-soft);
  padding: 10px 0;
  color: var(--text-muted);
  font-size: 13px;
}

.health-row:last-child {
  border-bottom: 0;
}

.health-row strong {
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.ok {
  color: var(--accent-green);
}

.warn {
  color: var(--accent-yellow);
}

.bad {
  color: var(--accent-red);
}

@media (max-width: 1120px) {
  .hardware-grid--top,
  .hardware-grid--bottom {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 560px) {
  .hardware-card {
    padding: 14px;
  }

  .storage-body {
    grid-template-columns: minmax(0, 1fr);
  }

  .storage-card .card-head {
    margin-bottom: 10px;
  }

  .donut {
    width: 144px;
    justify-self: center;
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
