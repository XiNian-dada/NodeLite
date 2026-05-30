<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { AgentLogEntry, LogLevel } from '@/api';

const props = defineProps<{ entries: AgentLogEntry[]; error: Error | null }>();

const { t, locale } = useI18n();

function levelLabelKey(level: LogLevel): string {
  if (level === 'warn') return 'node.logs.level_warn';
  if (level === 'error') return 'node.logs.level_error';
  return 'node.logs.level_info';
}

function fmtDateTime(value: string): string {
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? new Date(ms).toLocaleString(locale.value) : value;
}

// Newest first, matching legacy renderAgentLogs (reversed).
const rows = computed(() =>
  [...props.entries].reverse().map((entry, i) => ({
    // Stable-ish key from the timestamp; index disambiguates same-ms entries.
    key: `${entry.occurred_at}#${i}`,
    level: entry.level,
    levelLabel: t(levelLabelKey(entry.level)),
    time: fmtDateTime(entry.occurred_at),
    message: entry.message,
  })),
);
</script>

<template>
  <div class="log-stream" data-test="log-panel">
    <p v-if="error" class="placeholder" data-test="log-error">
      {{ t('node.logs.load_failed', { error: error.message }) }}
    </p>
    <p v-else-if="rows.length === 0" class="placeholder" data-test="log-empty">
      {{ t('node.logs.empty') }}
    </p>
    <article v-for="row in rows" v-else :key="row.key" class="log-entry" data-test="log-entry">
      <div class="log-meta">
        <span class="log-level" :class="row.level">{{ row.levelLabel }}</span>
        <span class="log-time">{{ row.time }}</span>
      </div>
      <pre class="log-text">{{ row.message }}</pre>
    </article>
  </div>
</template>

<style scoped>
.log-stream {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.placeholder {
  color: var(--text-muted);
  font-size: 13px;
  padding: 24px;
  margin: 0;
}
.log-entry {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 12px;
  padding: 10px 14px;
}
.log-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 4px;
  font-size: 12px;
  color: var(--text-muted);
}
.log-level {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  font-weight: 500;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
}
.log-level.warn {
  color: var(--accent-yellow);
  background: var(--accent-yellow-soft);
}
.log-level.error {
  color: var(--accent-red);
  background: var(--accent-red-soft);
}
.log-text {
  margin: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-primary);
}
</style>
