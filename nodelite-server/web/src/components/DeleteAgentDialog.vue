<script setup lang="ts">
import { reactive, ref, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient, type SettingsAgentToken } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import SettingsMessage from './SettingsMessage.vue';
import NativeDialog from './NativeDialog.vue';

const props = defineProps<{
  agent: SettingsAgentToken;
  twoFactorEnabled: boolean;
}>();
const emit = defineEmits<{ close: []; deleted: [] }>();
const { t } = useI18n();
const titleId = useId();

const deleting = ref(false);
const message = reactive<{ state: 'ok' | 'error' | null; text: string }>({
  state: null,
  text: '',
});

function close(): void {
  if (!deleting.value) emit('close');
}

async function deleteAgent(): Promise<void> {
  deleting.value = true;
  message.state = null;
  message.text = '';
  try {
    await apiClient.deleteAgent(props.agent.node_id, {});
    emit('deleted');
  } catch (error) {
    if (error instanceof ApiAbortError) return;
    message.state = 'error';
    message.text = t('settings.tokens.delete_failed', {
      error: messageFromError(error, 'unknown'),
    });
  } finally {
    deleting.value = false;
  }
}
</script>

<template>
  <NativeDialog
    class="delete-modal"
    data-test="delete-agent-modal"
    :labelled-by="titleId"
    :dismissible="!deleting"
    @close="close"
  >
    <form class="delete-modal__panel" data-test="delete-agent-form" @submit.prevent="deleteAgent">
      <header class="delete-modal__head">
        <div>
          <h3 :id="titleId">
            {{ t('settings.tokens.delete_title', { node: agent.node_label || agent.node_id }) }}
          </h3>
          <p>{{ agent.node_id }}</p>
        </div>
        <button
          class="delete-modal__close"
          type="button"
          :disabled="deleting"
          data-test="delete-agent-cancel"
          :aria-label="t('common.close')"
          autofocus
          @click="close"
        >
          ×
        </button>
      </header>
      <p class="delete-modal__warning">{{ t('settings.tokens.delete_warning') }}</p>
      <SettingsMessage :state="message.state" :text="message.text" />
      <footer class="delete-modal__actions">
        <button class="delete-modal__cancel" type="button" :disabled="deleting" @click="close">
          {{ t('settings.tokens.delete_cancel') }}
        </button>
        <button
          class="delete-modal__confirm"
          type="submit"
          :disabled="deleting"
          data-test="delete-agent-confirm"
        >
          {{
            deleting ? t('settings.tokens.delete_deleting') : t('settings.tokens.delete_confirm')
          }}
        </button>
      </footer>
    </form>
  </NativeDialog>
</template>

<style scoped>
.delete-modal {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.72);
}
.delete-modal__panel {
  width: min(440px, 100%);
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px;
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  box-shadow: var(--panel-shadow);
}
.delete-modal__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}
.delete-modal__head h3 {
  margin: 0;
  color: var(--text-primary);
  font-size: 16px;
}
.delete-modal__head p,
.delete-modal__warning {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.5;
}
.delete-modal__warning {
  color: var(--text-secondary);
}
.delete-modal__close,
.delete-modal__cancel,
.delete-modal__confirm {
  height: var(--btn-height, 32px);
  border-radius: var(--btn-radius, 8px);
  font: inherit;
  font-size: var(--btn-font-size, 13px);
  font-weight: 500;
  cursor: pointer;
  user-select: none;
  transition:
    background-color 0.15s ease,
    border-color 0.15s ease,
    color 0.15s ease;
}
.delete-modal__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  color: var(--text-muted);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  font-size: 18px;
  line-height: 1;
}
.delete-modal__close:hover:not(:disabled) {
  color: var(--text-primary);
  background: var(--bg-elevated);
  border-color: var(--border-strong);
}
.delete-modal__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--actions-gap, 10px);
}
.delete-modal__cancel {
  min-width: 64px;
  padding: 0 14px;
  color: var(--text-secondary);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
}
.delete-modal__cancel:hover:not(:disabled) {
  color: var(--text-primary);
  background: var(--bg-elevated);
  border-color: var(--border-strong);
}
.delete-modal__confirm {
  min-width: 96px;
  padding: 0 14px;
  color: var(--accent-red);
  background: var(--accent-red-soft);
  border: 1px solid transparent;
}
.delete-modal__confirm:hover:not(:disabled) {
  background: rgba(255, 77, 109, 0.28);
  color: #ff3358;
}
.delete-modal__close:disabled,
.delete-modal__cancel:disabled,
.delete-modal__confirm:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>
