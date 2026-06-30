<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { HistoryPoint, NodeStatus } from '@/api';
import { buildNodeNetworkModel } from '@/lib/nodeNetworkModel';
import MetricChart from './MetricChart.vue';

const props = defineProps<{ node: NodeStatus | null; history: HistoryPoint[] }>();

const { t } = useI18n();

const model = computed(() => buildNodeNetworkModel(props.node, props.history, t));
</script>

<template>
  <div class="network-panel" data-test="network-pane">
    <section class="network-stat-grid" data-test="network-stat-grid">
      <article
        v-for="card in model.statCards"
        :key="card.key"
        class="network-stat"
        :class="`network-stat--${card.tone}`"
        :data-test="`network-stat-${card.key}`"
      >
        <span class="network-stat__label">{{ card.label }}</span>
        <strong>{{ card.value }}</strong>
        <small>{{ card.meta }}</small>
      </article>
    </section>

    <section class="network-layout">
      <article class="network-card traffic-card" data-test="network-traffic-card">
        <header class="network-card__head">
          <div>
            <span class="card-kicker">{{ t('node.network.live') }}</span>
            <strong>{{ t('node.network_traffic') }}</strong>
          </div>
          <div class="traffic-legend" aria-hidden="true">
            <span class="legend-item legend-item--down">{{ t('index.node.download') }}</span>
            <span class="legend-item legend-item--up">{{ t('index.node.upload') }}</span>
          </div>
        </header>
        <MetricChart :series="model.trafficSeries" value-kind="rate" :min-value="0" :height="260" />
      </article>

      <article class="network-card quality-card" data-test="network-quality-card">
        <header class="network-card__head">
          <div>
            <span class="card-kicker">{{ t('node.network.quality') }}</span>
            <strong>{{ t('node.network.link_health') }}</strong>
          </div>
        </header>
        <div class="quality-meter" :class="`quality-meter--${model.packetLossTone}`">
          <span>{{ t('node.network.packet_loss') }}</span>
          <strong>{{ model.packetLossText }}</strong>
        </div>
        <div class="quality-list">
          <div v-for="row in model.qualityRows" :key="row.label" class="quality-row">
            <span>{{ row.label }}</span>
            <strong :class="`tone-${row.tone}`">{{ row.value }}</strong>
          </div>
        </div>
      </article>
    </section>

    <section class="network-layout network-layout--bottom">
      <article class="network-card loss-card" data-test="network-loss-card">
        <header class="network-card__head">
          <div>
            <span class="card-kicker">{{ t('node.network.loss_history') }}</span>
            <strong>{{ t('node.network.packet_loss') }}</strong>
          </div>
          <span class="head-value">{{ model.averagePacketLossText }}</span>
        </header>
        <MetricChart
          :points="model.chartData.packetLossPts"
          value-kind="percent"
          color="var(--accent-red)"
          :min-value="0"
          :max-value="100"
          :height="190"
          :label="t('node.network.packet_loss')"
        />
      </article>

      <article class="network-card totals-card" data-test="network-totals-card">
        <header class="network-card__head">
          <div>
            <span class="card-kicker">{{ t('node.network.totals') }}</span>
            <strong>{{ t('node.network.traffic_mix') }}</strong>
          </div>
          <span class="head-value">{{ model.totalTrafficText }}</span>
        </header>
        <div class="traffic-split" aria-hidden="true">
          <span class="traffic-split__rx" :style="{ width: `${model.rxShare}%` }" />
          <span class="traffic-split__tx" :style="{ width: `${model.txShare}%` }" />
        </div>
        <div class="totals-list">
          <div v-for="row in model.totalRows" :key="row.label" class="total-row">
            <span>{{ row.label }}</span>
            <strong>{{ row.value }}</strong>
          </div>
        </div>
      </article>
    </section>
  </div>
</template>

<style scoped>
.network-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.network-stat-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
}

.network-stat,
.network-card {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  box-shadow: var(--panel-shadow);
}

.network-stat {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 112px;
  padding: 16px;
  position: relative;
  overflow: hidden;
}

.network-stat::before {
  content: '';
  position: absolute;
  inset: 0;
  border-left: 3px solid var(--text-dim);
  opacity: 0.8;
  pointer-events: none;
}

.network-stat--ok::before {
  border-left-color: var(--accent-green);
}

.network-stat--warn::before {
  border-left-color: var(--accent-yellow);
}

.network-stat--bad::before {
  border-left-color: var(--accent-red);
}

.network-stat__label {
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 600;
}

.network-stat strong {
  color: var(--text-primary);
  font-size: 24px;
  font-variant-numeric: tabular-nums;
  font-weight: 650;
  line-height: 1.15;
}

.network-stat small {
  color: var(--text-muted);
  font-size: 12px;
  min-height: 18px;
}

.network-layout {
  display: grid;
  grid-template-columns: minmax(0, 1.65fr) minmax(260px, 0.85fr);
  gap: 16px;
}

.network-layout--bottom {
  grid-template-columns: minmax(0, 1.2fr) minmax(300px, 0.8fr);
}

.network-card {
  min-width: 0;
  padding: 16px;
}

.network-card__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
}

.network-card__head > div {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
}

.card-kicker {
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0;
  text-transform: uppercase;
}

.network-card__head strong {
  color: var(--text-primary);
  font-size: 16px;
  font-weight: 650;
}

.traffic-legend {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 10px;
  color: var(--text-muted);
  font-size: 12px;
}

.legend-item {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.legend-item::before {
  content: '';
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: currentColor;
}

.legend-item--down {
  color: var(--chart-network-down);
}

.legend-item--up {
  color: var(--chart-network-up);
}

.quality-card,
.totals-card {
  display: flex;
  flex-direction: column;
}

.quality-meter {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
  min-height: 102px;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  padding: 18px;
}

.quality-meter span {
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 600;
}

.quality-meter strong {
  font-size: 34px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
}

.quality-meter--ok strong {
  color: var(--accent-green);
}

.quality-meter--warn strong {
  color: var(--accent-yellow);
}

.quality-meter--bad strong {
  color: var(--accent-red);
}

.quality-list,
.totals-list {
  display: flex;
  flex-direction: column;
  margin-top: 14px;
}

.quality-row,
.total-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  border-top: 1px solid var(--border-soft);
  color: var(--text-muted);
  font-size: 13px;
  padding: 12px 0;
}

.quality-row strong,
.total-row strong {
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  font-weight: 650;
  text-align: right;
}

.tone-ok {
  color: var(--accent-green) !important;
}

.tone-warn {
  color: var(--accent-yellow) !important;
}

.tone-bad {
  color: var(--accent-red) !important;
}

.head-value {
  color: var(--text-secondary);
  flex: 0 0 auto;
  font-size: 18px;
  font-variant-numeric: tabular-nums;
  font-weight: 650;
}

.traffic-split {
  display: flex;
  width: 100%;
  height: 14px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
}

.traffic-split__rx {
  background: var(--chart-network-down);
}

.traffic-split__tx {
  background: var(--chart-network-up);
}

@media (max-width: 980px) {
  .network-stat-grid,
  .network-layout,
  .network-layout--bottom {
    grid-template-columns: 1fr 1fr;
  }

  .traffic-card,
  .loss-card {
    grid-column: 1 / -1;
  }
}

@media (max-width: 620px) {
  .network-stat-grid,
  .network-layout,
  .network-layout--bottom {
    grid-template-columns: 1fr;
  }

  .network-stat strong {
    font-size: 22px;
  }

  .network-card__head {
    flex-direction: column;
  }
}
</style>
