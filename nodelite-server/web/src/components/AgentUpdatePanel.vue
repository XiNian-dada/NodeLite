<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient } from '@/api';
import { ApiAbortError } from '@/api/client';
import type { AgentUpdateView } from '@/api/types';
import { messageFromError } from '@/lib/apiError';
import NativeDialog from './NativeDialog.vue';

const props = defineProps<{ nodeId: string }>();
const { t } = useI18n();
const titleId = useId();
const view = ref<AgentUpdateView | null>(null);
const requestError = ref('');
const loadError = ref('');
const error = computed(() => requestError.value || loadError.value);
const busy = ref(false);
const confirming = ref(false);
const progress = computed(
  () =>
    view.value &&
    ['queued', 'downloading', 'prepared', 'stopping', 'trial', 'rolling_back'].includes(
      view.value.status.phase,
    ),
);
const explanation = computed(() => {
  if (!view.value) return '';
  if (view.value.status.reason) return t(`agent_update.reason.${view.value.status.reason}`);
  if (!view.value.online) return t('agent_update.offline');
  if (!view.value.can_update && !progress.value) return t('agent_update.cooldown');
  return '';
});

watch(
  () => props.nodeId,
  (nodeId, _previous, onCleanup) => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    view.value = null;
    requestError.value = '';
    loadError.value = '';
    confirming.value = false;
    async function poll(): Promise<void> {
      try {
        const result = await apiClient.agentUpdate(nodeId, controller.signal);
        if (!controller.signal.aborted) {
          view.value = result;
          loadError.value = '';
        }
      } catch (cause) {
        if (!controller.signal.aborted && !(cause instanceof ApiAbortError)) {
          loadError.value = messageFromError(cause, t('agent_update.load_failed'));
        }
      } finally {
        if (!controller.signal.aborted) timer = setTimeout(poll, 2000);
      }
    }
    void poll();
    onCleanup(() => {
      controller.abort();
      clearTimeout(timer);
    });
  },
  { immediate: true },
);

async function upgrade(): Promise<void> {
  const nodeId = props.nodeId;
  confirming.value = false;
  busy.value = true;
  requestError.value = '';
  try {
    const result = await apiClient.startAgentUpdate(nodeId);
    if (props.nodeId === nodeId) view.value = result;
  } catch (cause) {
    if (props.nodeId === nodeId && !(cause instanceof ApiAbortError)) {
      requestError.value = messageFromError(cause, t('agent_update.request_failed'));
    }
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <article class="panel agent-update" data-test="agent-update-panel">
    <header class="card-head">
      <h2 class="card-title">{{ t('agent_update.title') }}</h2>
    </header>
    <p class="agent-update__hint">{{ t('agent_update.hint') }}</p>
    <template v-if="view">
      <dl class="agent-update__details">
        <div>
          <dt>{{ t('agent_update.version') }}</dt>
          <dd data-test="agent-update-version">{{ view.agent_version }}</dd>
        </div>
        <div v-if="view.status.channel">
          <dt>{{ t('agent_update.channel') }}</dt>
          <dd>{{ t(`agent_update.channel.${view.status.channel}`) }}</dd>
        </div>
        <div>
          <dt>{{ t('agent_update.status') }}</dt>
          <dd
            role="status"
            aria-live="polite"
            data-test="agent-update-phase"
            :data-phase="view.status.phase"
          >
            {{ t(`agent_update.phase.${view.status.phase}`) }}
          </dd>
        </div>
      </dl>
      <p v-if="explanation" class="agent-update__hint" data-test="agent-update-reason">
        {{ explanation }}
      </p>
    </template>
    <p v-else>{{ t('common.waiting_for_data') }}</p>
    <p v-if="error" role="alert" class="agent-update__error" data-test="agent-update-error">
      {{ error }}
    </p>
    <button
      type="button"
      class="btn btn--primary"
      data-test="agent-update-start"
      :disabled="!view?.can_update || busy || !!loadError"
      @click="confirming = true"
    >
      {{ t(busy || progress ? 'agent_update.working' : 'agent_update.start') }}
    </button>
    <NativeDialog
      v-if="confirming"
      class="agent-update__overlay"
      :labelled-by="titleId"
      @close="confirming = false"
    >
      <section class="agent-update__confirm" data-test="agent-update-confirm">
        <h2 :id="titleId">{{ t('agent_update.confirm_title') }}</h2>
        <p>{{ t('agent_update.confirm_note') }}</p>
        <div class="agent-update__actions">
          <button
            type="button"
            class="btn btn--subtle"
            data-test="agent-update-cancel"
            @click="confirming = false"
          >
            {{ t('agent_update.cancel') }}
          </button>
          <button
            type="button"
            class="btn btn--primary"
            data-test="agent-update-confirm-submit"
            @click="upgrade"
          >
            {{ t('agent_update.confirm') }}
          </button>
        </div>
      </section>
    </NativeDialog>
  </article>
</template>

<style scoped>
.agent-update {
  display: grid;
  gap: 1rem;
}
.agent-update__hint {
  margin: 0;
  color: var(--text-muted);
  line-height: 1.6;
}
.agent-update__details {
  display: grid;
  gap: 0.75rem;
  margin: 0;
}
.agent-update__details > div {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
}
.agent-update__details dt {
  color: var(--text-muted);
}
.agent-update__details dd {
  margin: 0;
  text-align: right;
  overflow-wrap: anywhere;
}
.agent-update__error {
  color: var(--accent-red);
  overflow-wrap: anywhere;
}
.agent-update__overlay {
  background: rgb(0 0 0 / 45%);
}
.agent-update__confirm {
  width: min(32rem, calc(100% - 3rem));
  margin: 15vh auto;
  padding: 1.5rem;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-lg, 12px);
  background: var(--bg-card);
  line-height: 1.6;
}
.agent-update__actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 1.5rem;
}
</style>
