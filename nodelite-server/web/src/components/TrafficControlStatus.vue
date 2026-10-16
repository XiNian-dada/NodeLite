<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { TrafficControlStatus } from '@/api/types';

const props = withDefaults(
  defineProps<{ status?: TrafficControlStatus | null; compact?: boolean }>(),
  {
    status: null,
    compact: false,
  },
);
const { t } = useI18n({
  useScope: 'local',
  messages: {
    en: {
      label: 'Traffic control',
      details: 'Details',
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
      details: '查看原因',
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
    <div class="traffic-status__heading">
      <span>{{ t('label') }}</span>
      <strong>{{ summary }}</strong>
    </div>
    <details v-if="compact && (status?.reason || status?.desired_rate_kbps != null)">
      <summary>{{ t('details') }}</summary>
      <p v-if="status?.reason">{{ t(status.reason) }}</p>
      <p v-if="status?.desired_rate_kbps != null">
        {{ t('desired', { rate: status.desired_rate_kbps }) }}
      </p>
    </details>
    <template v-else>
      <p v-if="status?.reason">{{ t(status.reason) }}</p>
      <p v-if="status?.desired_rate_kbps != null">
        {{ t('desired', { rate: status.desired_rate_kbps }) }}
      </p>
    </template>
  </div>
</template>

<style scoped>
.traffic-status {
  margin-top: 6px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-muted);
}
.traffic-status__heading {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 4px 8px;
}
.traffic-status strong {
  font-weight: 500;
  color: var(--text-secondary);
}
.traffic-status p {
  margin: 5px 0 0;
  overflow-wrap: anywhere;
}
.traffic-status summary {
  cursor: pointer;
  width: fit-content;
  margin-top: 5px;
  color: var(--text-secondary);
}
.traffic-status[data-state='ready'] strong,
.traffic-status[data-state='applied'] strong {
  color: var(--accent-green);
}
.traffic-status[data-state='unavailable'] strong,
.traffic-status[data-state='retrying'] strong {
  color: var(--accent-yellow);
}
.traffic-status[data-state='failed'] strong {
  color: var(--accent-red);
}
</style>
