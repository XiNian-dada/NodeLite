<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { NodeListItem } from '@/api';
import { useNodesStore } from '@/stores/nodes';
import { useWorldGeoJson } from '@/composables/useWorldGeoJson';
import { useTheme } from '@/composables/useTheme';
import { nodePosition, nodeStatusKey } from '@/lib/map/projection';
import { drawFallbackMask, drawGeoJsonMask, paintWorldDotMap } from '@/lib/map/landMask';
import { locationFromNode } from '@/lib/nodeMeta';

const nodesStore = useNodesStore();
const { geojson, load } = useWorldGeoJson();
const { theme } = useTheme();
const { t } = useI18n();

const canvas = ref<HTMLCanvasElement | null>(null);
const activeDotId = ref<string | null>(null);

const CROWD_THRESHOLD = 3;

const dots = computed(() => {
  const positions = nodesStore.nodes.map((node) => {
    const pos = nodePosition(node);
    return { x: pos.x * 100, y: pos.y * 100 };
  });

  return nodesStore.nodes.map((node, i) => {
    const { x, y } = positions[i]!;
    const crowded = positions.some(
      (other, j) =>
        j !== i &&
        Math.abs(other.x - x) < CROWD_THRESHOLD &&
        Math.abs(other.y - y) < CROWD_THRESHOLD,
    );
    return {
      id: node.identity.node_id,
      label: node.identity.node_label || node.identity.node_id,
      status: nodeStatusKey(node),
      statusLabelKey: statusLabelKey(node),
      location: locationText(node),
      load: node.snapshot?.load.one == null ? '—' : node.snapshot.load.one.toFixed(2),
      latency: node.latency_ms == null ? '—' : `${Math.round(node.latency_ms)} ms`,
      left: `${x.toFixed(2)}%`,
      top: `${y.toFixed(2)}%`,
      edgeX: x > 72 ? 'right' : x < 28 ? 'left' : 'center',
      edgeY: y < 28 ? 'bottom' : 'top',
      crowded,
    };
  });
});

const activeDot = computed(() => dots.value.find((dot) => dot.id === activeDotId.value) ?? null);

function statusLabelKey(node: NodeListItem): string {
  switch (nodeStatusKey(node)) {
    case 'offline':
      return 'common.offline';
    case 'latency':
      return 'common.latency_warn';
    default:
      return 'common.online';
  }
}

function locationText(node: NodeListItem): string {
  return locationFromNode(node) || node.identity.hostname;
}

function dotColor(): string {
  const el = canvas.value ?? document.documentElement;
  const value = getComputedStyle(el).getPropertyValue('--map-land-dot').trim();
  return value || 'rgba(148,163,184,0.58)';
}

function repaint(): void {
  const el = canvas.value;
  if (!el) return;
  const geo = geojson.value;
  paintWorldDotMap(el, geo ? (ctx) => drawGeoJsonMask(ctx, geo) : drawFallbackMask, dotColor());
}

onMounted(() => {
  // Paint the built-in fallback immediately, then upgrade to the fetched
  // GeoJSON when it arrives. A fetch failure simply keeps the fallback.
  repaint();
  void load();
});

// Re-render the land mask when the real GeoJSON resolves, and again on
// theme change — the land-dot colour comes from the --map-land-dot CSS var,
// which is read at paint time (matches legacy's repaint-on-toggle).
watch([geojson, theme], repaint);
</script>

<template>
  <article class="panel map-card" data-test="node-map">
    <div class="panel-head">
      <div class="panel-title">
        <span>{{ $t('index.map.title') }}</span>
      </div>
      <div class="map-legend">
        <span class="legend-online">
          <span class="legend-swatch" />{{ $t('index.map.legend_online') }}
        </span>
        <span class="legend-latency">
          <span class="legend-swatch" />{{ $t('index.map.legend_latency') }}
        </span>
        <span class="legend-offline">
          <span class="legend-swatch" />{{ $t('index.map.legend_offline') }}
        </span>
      </div>
    </div>
    <div class="map-stage">
      <div class="map-grid" />
      <canvas ref="canvas" class="map-canvas" width="1200" height="600" aria-hidden="true" />
      <div class="map-dots" data-test="map-dots">
        <div
          v-for="dot in dots"
          :key="dot.id"
          class="map-dot"
          :class="[dot.status, { 'map-dot--small': dot.crowded }]"
          :style="{ left: dot.left, top: dot.top }"
          :title="dot.label"
          tabindex="0"
          data-test="map-dot"
          @pointerenter="activeDotId = dot.id"
          @pointerleave="activeDotId = null"
          @focus="activeDotId = dot.id"
          @blur="activeDotId = null"
        />
      </div>
      <div
        v-if="activeDot"
        class="map-hover-card"
        :class="[
          `map-hover-card--x-${activeDot.edgeX}`,
          `map-hover-card--y-${activeDot.edgeY}`,
        ]"
        :style="{ left: activeDot.left, top: activeDot.top }"
        data-test="map-hover-card"
      >
        <div class="map-hover-card__head">
          <span class="map-hover-card__title">{{ activeDot.label }}</span>
          <span class="map-hover-card__status" :class="activeDot.status">
            {{ t(activeDot.statusLabelKey) }}
          </span>
        </div>
        <div class="map-hover-card__location">{{ activeDot.location }}</div>
        <div class="map-hover-card__metrics">
          <span>
            <small>{{ t('index.node.load') }}</small>
            <strong>{{ activeDot.load }}</strong>
          </span>
          <span>
            <small>{{ t('index.node.latency') }}</small>
            <strong>{{ activeDot.latency }}</strong>
          </span>
        </div>
      </div>
    </div>
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  box-shadow: var(--panel-shadow);
  padding: 18px 20px;
}
.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 14px;
}
.panel-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-secondary);
}
.map-legend {
  display: flex;
  gap: 14px;
  flex-wrap: wrap;
  color: var(--text-muted);
  font-size: 12px;
}
.map-legend span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.legend-swatch {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.legend-online .legend-swatch {
  background: var(--accent-green);
}
.legend-latency .legend-swatch {
  background: var(--accent-yellow);
}
.legend-offline .legend-swatch {
  background: var(--accent-red);
}
.map-stage {
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 8;
  min-height: 280px;
  background: linear-gradient(var(--map-stage-sheen), transparent), var(--map-stage-bg);
  border: 1px solid var(--map-stage-border);
  border-radius: 8px;
  overflow: hidden;
}
@media (max-width: 640px) {
  .map-stage {
    aspect-ratio: 16 / 10;
    min-height: 0;
    height: clamp(160px, 45vw, 200px);
  }
}
.map-grid {
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(var(--map-grid-line) 1px, transparent 1px),
    linear-gradient(90deg, var(--map-grid-line) 1px, transparent 1px);
  background-size: 5% 10%;
}
.map-canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  filter: drop-shadow(0 18px 36px rgba(0, 0, 0, 0.2));
}
.map-dots {
  position: absolute;
  inset: 0;
}
.map-dot {
  position: absolute;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  transform: translate(-50%, -50%);
  outline: none;
  box-shadow:
    0 0 0 3px var(--map-dot-ring),
    0 0 18px currentColor;
}
.map-dot--small {
  width: 4px;
  height: 4px;
  box-shadow:
    0 0 0 2px var(--map-dot-ring),
    0 0 12px currentColor;
}
.map-dot--small::after {
  inset: -3px;
  animation-name: dotPulseSmall;
}
@keyframes dotPulseSmall {
  0% {
    transform: scale(0.6);
    opacity: 0.35;
  }
  100% {
    transform: scale(1.3);
    opacity: 0;
  }
}
.map-dot:hover,
.map-dot:focus-visible {
  z-index: 3;
}
.map-dot::after {
  content: '';
  position: absolute;
  inset: -5px;
  border-radius: 50%;
  animation: dotPulse 2.4s infinite ease-out;
  background: currentColor;
  opacity: 0.14;
}
.map-dot.online {
  color: var(--accent-green);
  background: var(--accent-green);
}
.map-dot.latency {
  color: var(--accent-yellow);
  background: var(--accent-yellow);
}
.map-dot.offline {
  color: var(--accent-red);
  background: var(--accent-red);
}
.map-hover-card {
  position: absolute;
  z-index: 4;
  width: min(240px, calc(100% - 24px));
  padding: 12px 13px;
  color: var(--text-primary);
  background: color-mix(in srgb, var(--bg-card) 94%, transparent);
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  box-shadow: 0 18px 42px rgba(0, 0, 0, 0.28);
  pointer-events: none;
}
.map-hover-card--x-center.map-hover-card--y-top {
  transform: translate(-50%, calc(-100% - 16px));
}
.map-hover-card--x-left.map-hover-card--y-top {
  transform: translate(-12px, calc(-100% - 16px));
}
.map-hover-card--x-right.map-hover-card--y-top {
  transform: translate(calc(-100% + 12px), calc(-100% - 16px));
}
.map-hover-card--x-center.map-hover-card--y-bottom {
  transform: translate(-50%, 16px);
}
.map-hover-card--x-left.map-hover-card--y-bottom {
  transform: translate(-12px, 16px);
}
.map-hover-card--x-right.map-hover-card--y-bottom {
  transform: translate(calc(-100% + 12px), 16px);
}
.map-hover-card__head,
.map-hover-card__metrics {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.map-hover-card__title {
  min-width: 0;
  overflow: hidden;
  color: var(--text-primary);
  font-size: 13px;
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.map-hover-card__status {
  flex: 0 0 auto;
  font-size: 11px;
  font-weight: 600;
}
.map-hover-card__status.online {
  color: var(--accent-green);
}
.map-hover-card__status.latency {
  color: var(--accent-yellow);
}
.map-hover-card__status.offline {
  color: var(--accent-red);
}
.map-hover-card__location {
  margin-top: 5px;
  overflow: hidden;
  color: var(--text-muted);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.map-hover-card__metrics {
  margin-top: 12px;
}
.map-hover-card__metrics span {
  display: grid;
  gap: 3px;
}
.map-hover-card__metrics small {
  color: var(--text-muted);
  font-size: 10px;
}
.map-hover-card__metrics strong {
  color: var(--text-primary);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
}
@keyframes dotPulse {
  0% {
    transform: scale(0.6);
    opacity: 0.45;
  }
  100% {
    transform: scale(1.6);
    opacity: 0;
  }
}
</style>
