<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { TrafficControlStatus } from '@/api/types';

const props = defineProps<{ status?: TrafficControlStatus | null }>();
const { t } = useI18n({
  useScope: 'local',
  messages: {
    en: {
      label: 'Traffic control',
      unknown: 'Not reported',
      ready: 'Available',
      applied: 'Applied: {rate} kbit/s',
      cleared: 'No limit applied',
      failed: 'Apply failed; retry limit reached',
      retrying: 'Apply failed; retry scheduled',
      unavailable: 'Unavailable',
      desired: 'Requested: {rate} kbit/s',
      disabled: 'Enable traffic control when installing the Agent',
      unsupported_platform: 'Unsupported platform',
      missing_tc: 'tc is not installed',
      missing_capability: 'Network management permission is missing',
    },
    'zh-CN': {
      label: '限速能力',
      unknown: '尚未上报',
      ready: '可用',
      applied: '已应用：{rate} kbit/s',
      cleared: '未应用限速',
      failed: '执行失败，已达到重试上限',
      retrying: '执行失败，将自动重试',
      unavailable: '不可用',
      desired: '期望速率：{rate} kbit/s',
      disabled: '请在安装 Agent 时启用限速能力',
      unsupported_platform: '当前平台不支持',
      missing_tc: '尚未安装 tc',
      missing_capability: '缺少网络管理权限',
    },
  },
});
const summary = computed(() => {
  const status = props.status;
  if (!status) return t('unknown');
  if (status.state === 'applied') {
    return status.applied_rate_kbps === null
      ? t('cleared')
      : t('applied', { rate: status.applied_rate_kbps });
  }
  return t(status.state);
});
</script>

<template>
  <div
    class="traffic-status"
    data-test="traffic-control-status"
    :data-state="status?.state ?? 'unknown'"
  >
    <div>{{ t('label') }}: {{ summary }}</div>
    <div v-if="status?.reason">{{ t(status.reason) }}</div>
    <div v-if="status?.desired_rate_kbps != null">
      {{ t('desired', { rate: status.desired_rate_kbps }) }}
    </div>
  </div>
</template>

<style scoped>
.traffic-status {
  margin-top: 6px;
  font-size: 11px;
  color: var(--text-muted);
}
.traffic-status[data-state='unavailable'],
.traffic-status[data-state='failed'] {
  color: var(--accent-red);
}
</style>
