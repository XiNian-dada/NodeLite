<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { useRealtimeStore } from '@/stores/realtime';

const realtime = useRealtimeStore();
const { t } = useI18n({
  useScope: 'local',
  messages: {
    en: {
      polling: 'Live connection interrupted; refreshing periodically.',
      stale: 'Data may be out of date. Retrying automatically.',
    },
    'zh-CN': { polling: '实时连接中断，正在定时刷新。', stale: '数据可能已过期，正在自动重试。' },
  },
});
</script>

<template>
  <p
    v-if="!realtime.streaming"
    role="status"
    class="connection-status"
    data-test="connection-status"
    :data-stale="realtime.stale"
  >
    {{ t(realtime.stale ? 'stale' : 'polling') }}
  </p>
</template>

<style scoped>
.connection-status {
  color: var(--text-muted);
  font-size: 13px;
  margin: 0 0 16px;
}
.connection-status[data-stale='true'] {
  color: var(--accent-yellow, #b7791f);
}
</style>
