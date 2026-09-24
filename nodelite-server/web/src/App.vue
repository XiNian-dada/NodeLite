<script setup lang="ts">
import { useRealtimeData } from '@/composables/useRealtimeData';
import { stepUpOpen } from '@/auth/stepUp';
import StepUpDialog from '@/components/StepUpDialog.vue';

useRealtimeData();
</script>

<template>
  <RouterView />
  <StepUpDialog v-if="stepUpOpen" />
</template>
