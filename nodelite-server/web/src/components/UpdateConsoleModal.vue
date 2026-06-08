<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import { fmtBytes } from '@/lib/format';

type ConsoleStatus = 'idle' | 'waiting' | 'running' | 'error';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();

const statusKind = ref<ConsoleStatus>('idle');
const statusText = ref(t('settings.version.console_status_idle'));
const metaText = ref(t('settings.version.console_empty'));
const followText = ref(t('settings.version.console_following'));
const logText = ref(t('settings.version.console_empty'));
const logEl = ref<HTMLElement | null>(null);

const offset = ref(0);
const lastRenderedText = ref('');
const lastError = ref('');
let timer: number | null = null;

function clearPollTimer(): void {
  if (timer == null) return;
  window.clearTimeout(timer);
  timer = null;
}

function schedulePoll(delayMs = 1600): void {
  if (!props.open) return;
  clearPollTimer();
  timer = window.setTimeout(() => {
    void fetchLog();
  }, delayMs);
}

function isNearBottom(): boolean {
  const el = logEl.value;
  return !el || el.scrollHeight - el.scrollTop - el.clientHeight < 36;
}

async function scrollToBottom(): Promise<void> {
  await nextTick();
  const el = logEl.value;
  if (el) el.scrollTop = el.scrollHeight;
}

function setStatus(kind: ConsoleStatus, text: string): void {
  statusKind.value = kind;
  statusText.value = text;
}

function setMeta(text: string): void {
  metaText.value = text;
}

function setText(text: string): void {
  logText.value = text;
  lastRenderedText.value = text;
}

function appendLine(text: string): void {
  const shouldFollow = isNearBottom();
  const next = logText.value && lastRenderedText.value ? `${logText.value}\n${text}` : text;
  logText.value = next;
  lastRenderedText.value = next;
  if (shouldFollow) void scrollToBottom();
}

function reset(): void {
  clearPollTimer();
  offset.value = 0;
  lastError.value = '';
  setText('');
}

async function fetchLog(options: { reset?: boolean; silent?: boolean } = {}): Promise<void> {
  if (!props.open) return;
  const shouldReset = options.reset ?? false;
  if (shouldReset) reset();

  const shouldFollow = isNearBottom();
  followText.value = shouldFollow
    ? t('settings.version.console_following')
    : t('settings.version.console_paused');

  try {
    const body = await apiClient.serverUpdateLog(offset.value);
    if (!body.exists) {
      if (!options.silent) {
        setStatus('idle', t('settings.version.console_status_idle'));
        setMeta(t('settings.version.console_empty'));
        if (!lastRenderedText.value) setText(t('settings.version.console_empty'));
      }
      schedulePoll(2500);
      return;
    }

    const text = String(body.text || '');
    const resetRequired = shouldReset || Number(body.offset || 0) < offset.value;
    if (resetRequired) {
      setText(text);
    } else if (text) {
      appendLine(text);
    }

    offset.value = Number(body.next_offset || 0);
    lastError.value = '';
    setStatus('running', t('settings.version.console_status_running'));
    setMeta(
      t('settings.version.console_loaded', { size: fmtBytes(offset.value) ?? `${offset.value} B` }),
    );
    if (shouldFollow) void scrollToBottom();
    schedulePoll(1500);
  } catch (error) {
    if (error instanceof ApiAbortError) return;
    const message = messageFromError(error, 'unknown');
    setStatus('error', t('settings.version.console_status_retrying'));
    setMeta(t('settings.version.console_fetch_failed', { error: message }));
    const line = `[client] ${t('settings.version.console_fetch_failed', { error: message })}`;
    if (line !== lastError.value) {
      appendLine(line);
      lastError.value = line;
    }
    schedulePoll(3000);
  }
}

function close(): void {
  clearPollTimer();
  emit('close');
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      clearPollTimer();
      return;
    }
    if (!lastRenderedText.value) {
      setStatus('idle', t('settings.version.console_status_idle'));
      setMeta(t('settings.version.console_empty'));
      setText(t('settings.version.console_empty'));
    }
  },
);

onBeforeUnmount(clearPollTimer);

defineExpose({
  appendLine,
  fetchLog,
  reset,
  setMeta,
  setStatus,
  setText,
});
</script>

<template>
  <div
    v-if="open"
    class="update-console"
    data-test="update-console-modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="update-console-title"
    @click.self="close"
  >
    <section class="update-console__panel">
      <header class="update-console__head">
        <div class="update-console__title">
          <h2 id="update-console-title">{{ t('settings.version.console_title') }}</h2>
          <p>{{ t('settings.version.console_subtitle') }}</p>
        </div>
        <div class="update-console__actions">
          <span class="update-console__status" :class="`update-console__status--${statusKind}`">
            {{ statusText }}
          </span>
          <button
            type="button"
            class="update-console__button"
            data-test="update-console-refresh"
            @click="fetchLog({ reset: true })"
          >
            {{ t('settings.version.console_refresh') }}
          </button>
          <button
            type="button"
            class="update-console__button"
            data-test="update-console-close"
            @click="close"
          >
            {{ t('settings.version.console_close') }}
          </button>
        </div>
      </header>
      <div class="update-console__toolbar">
        <span>{{ metaText }}</span>
        <span>{{ followText }}</span>
      </div>
      <div class="update-console__body">
        <pre ref="logEl" class="update-console__log" data-test="update-console-log">{{
          logText
        }}</pre>
      </div>
    </section>
  </div>
</template>

<style scoped>
.update-console {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.72);
}
.update-console__panel {
  width: min(880px, 100%);
  max-height: calc(100vh - 48px);
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr);
  gap: 12px;
  overflow: hidden;
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  box-shadow: var(--panel-shadow);
}
.update-console__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 16px 16px 0;
}
.update-console__title {
  min-width: 0;
}
.update-console__title h2 {
  margin: 0;
  color: var(--text-primary);
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 0;
}
.update-console__title p {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 12px;
}
.update-console__actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  flex-wrap: wrap;
}
.update-console__status {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 34px;
  padding: 0 10px;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.update-console__status::before {
  content: '';
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
}
.update-console__status--running {
  color: var(--accent-green);
}
.update-console__status--waiting {
  color: var(--accent-blue);
}
.update-console__status--error {
  color: var(--accent-red);
}
.update-console__button {
  height: 34px;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  padding: 0 12px;
}
.update-console__button:hover {
  color: var(--text-primary);
}
.update-console__toolbar {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 0 16px;
  color: var(--text-muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.update-console__body {
  min-height: 260px;
  margin: 0 16px 16px;
  overflow: hidden;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
}
.update-console__log {
  height: 100%;
  min-height: 260px;
  margin: 0;
  overflow: auto;
  padding: 14px;
  color: var(--text-secondary);
  font:
    13px/1.6 ui-monospace,
    SFMono-Regular,
    Menlo,
    Monaco,
    Consolas,
    monospace;
  white-space: pre-wrap;
  word-break: break-word;
  tab-size: 2;
}
@media (max-width: 720px) {
  .update-console {
    padding: 12px;
  }
  .update-console__head {
    flex-direction: column;
    align-items: stretch;
  }
  .update-console__actions {
    justify-content: flex-start;
  }
  .update-console__toolbar {
    flex-direction: column;
    gap: 4px;
  }
}
</style>
