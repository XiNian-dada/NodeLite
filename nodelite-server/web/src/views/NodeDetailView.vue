<script setup lang="ts">
import { useRoute } from 'vue-router';

const route = useRoute();
</script>

<template>
  <section class="view view--node" data-test="node-detail-view">
    <h1>Node: {{ route.params.id }}</h1>
    <p>Stage 3 will replace this placeholder with the full detail view.</p>
  </section>
</template>

<style scoped>
.view--node {
  padding: 24px;
  color: var(--text-primary);
}
</style>
