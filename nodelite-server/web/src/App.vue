<script setup lang="ts">
import { useRealtimeData } from '@/composables/useRealtimeData';

useRealtimeData();
</script>

<template>
  <RouterView />
</template>
