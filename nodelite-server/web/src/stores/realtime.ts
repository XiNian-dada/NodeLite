import { defineStore } from 'pinia';
import { computed, ref, shallowRef } from 'vue';
import type { ConnectionState } from '@/ws/client';

export const useRealtimeStore = defineStore('realtime', () => {
  const connection = shallowRef<ConnectionState>({ kind: 'idle' });
  const streaming = ref(false);
  const lastUpdatedAt = ref<number | null>(null);
  const now = ref(Date.now());
  const stale = computed(
    () =>
      !streaming.value &&
      (lastUpdatedAt.value === null || now.value - lastUpdatedAt.value > 15_000),
  );
  return { connection, streaming, lastUpdatedAt, now, stale };
});
