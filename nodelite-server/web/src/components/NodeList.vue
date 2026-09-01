<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import {
  gridColumnCount,
  gridVirtualWindow,
  NODE_CARD_HEIGHT,
  NODE_GRID_GAP,
} from '@/lib/gridVirtualizer';
import { useNodesStore } from '@/stores/nodes';
import NodeCard from './NodeCard.vue';

const nodesStore = useNodesStore();
const section = ref<HTMLElement | null>(null);
const listWidth = ref(window.innerWidth);
const viewportWidth = ref(window.innerWidth);
const viewportHeight = ref(window.innerHeight);
const scrollY = ref(window.scrollY);
const sectionTop = ref(0);
let scrollFrame: number | null = null;
let resizeObserver: ResizeObserver | null = null;

const columns = computed(() => gridColumnCount(listWidth.value, viewportWidth.value));
const virtualWindow = computed(() =>
  gridVirtualWindow(
    nodesStore.nodes.length,
    columns.value,
    scrollY.value - sectionTop.value,
    viewportHeight.value,
  ),
);
const visibleNodes = computed(() =>
  nodesStore.nodes.slice(virtualWindow.value.startIndex, virtualWindow.value.endIndex),
);
const spacerStyle = computed(() => ({ height: `${virtualWindow.value.totalHeight}px` }));
const gridStyle = computed(() => ({
  '--node-grid-columns': String(columns.value),
  '--node-card-height': `${NODE_CARD_HEIGHT}px`,
  '--node-grid-gap': `${NODE_GRID_GAP}px`,
  transform: `translate3d(0, ${virtualWindow.value.offsetTop}px, 0)`,
}));

function measureLayout(): void {
  const element = section.value;
  if (!element) return;
  const rect = element.getBoundingClientRect();
  listWidth.value = rect.width || element.clientWidth || window.innerWidth;
  viewportWidth.value = window.innerWidth;
  viewportHeight.value = window.innerHeight;
  scrollY.value = window.scrollY;
  sectionTop.value = rect.top + window.scrollY;
}

function updateScrollPosition(): void {
  if (scrollFrame !== null) return;
  scrollFrame = window.requestAnimationFrame(() => {
    scrollFrame = null;
    scrollY.value = window.scrollY;
  });
}

onMounted(() => {
  measureLayout();
  window.addEventListener('scroll', updateScrollPosition, { passive: true });
  window.addEventListener('resize', measureLayout);
  if (typeof ResizeObserver !== 'undefined') {
    resizeObserver = new ResizeObserver(measureLayout);
    const element = section.value;
    if (element) resizeObserver.observe(element);
  }
});

onUnmounted(() => {
  window.removeEventListener('scroll', updateScrollPosition);
  window.removeEventListener('resize', measureLayout);
  resizeObserver?.disconnect();
  if (scrollFrame !== null) window.cancelAnimationFrame(scrollFrame);
});
</script>

<template>
  <section ref="section" class="nodes-section" data-test="node-list">
    <div
      v-if="nodesStore.nodes.length > 0"
      class="node-grid-spacer"
      data-test="node-grid-spacer"
      :style="spacerStyle"
    >
      <div
        class="node-grid"
        data-test="node-grid-window"
        :data-start-index="virtualWindow.startIndex"
        :style="gridStyle"
      >
        <NodeCard v-for="node in visibleNodes" :key="node.identity.node_id" :node="node" />
      </div>
    </div>
    <p v-else class="nodes-empty" data-test="node-list-empty">
      {{ $t('common.waiting_for_data') }}
    </p>
  </section>
</template>

<style scoped>
.nodes-section {
  margin-top: 0;
}
.node-grid-spacer {
  position: relative;
  width: 100%;
}
.node-grid {
  position: absolute;
  inset: 0 0 auto;
  display: grid;
  grid-template-columns: repeat(var(--node-grid-columns), minmax(0, 1fr));
  gap: var(--node-grid-gap);
  will-change: transform;
}
.node-grid :deep(.node-card) {
  height: var(--node-card-height);
  min-height: var(--node-card-height);
}
.nodes-empty {
  color: var(--text-muted);
  font-size: 13px;
  margin: 0;
  padding: 24px 0;
}
</style>
