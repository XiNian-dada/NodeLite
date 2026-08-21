<script setup lang="ts">
import { reactive, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient, type SettingsAgentToken } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import ReauthFields from './ReauthFields.vue';
import SettingsMessage from './SettingsMessage.vue';

const props = defineProps<{
  agent: SettingsAgentToken;
  twoFactorEnabled: boolean;
}>();
const emit = defineEmits<{ close: []; deleted: [] }>();
const { t } = useI18n();

const deleting = ref(false);
const reauth = reactive({ currentPassword: '', code: '' });
const message = reactive<{ state: 'ok' | 'error' | null; text: string }>({
  state: null,
  text: '',
});

function close(): void {
  if (!deleting.value) emit('close');
}

function confirmationPayload() {
  return props.twoFactorEnabled
    ? { code: reauth.code }
    : { current_password: reauth.currentPassword };
}

async function deleteAgent(): Promise<void> {
  deleting.value = true;
  message.state = null;
  message.text = '';
  try {
    await apiClient.deleteAgent(props.agent.node_id, confirmationPayload());
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
  <div
    class="delete-modal"
    data-test="delete-agent-modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="delete-agent-title"
    @click.self="close"
  >
    <form class="delete-modal__panel" data-test="delete-agent-form" @submit.prevent="deleteAgent">
      <header class="delete-modal__head">
        <div>
          <h3 id="delete-agent-title">
            {{ t('settings.tokens.delete_title', { node: agent.node_label || agent.node_id }) }}
          </h3>
          <p>{{ agent.node_id }}</p>
        </div>
        <button
          class="delete-modal__close"
          type="button"
          :disabled="deleting"
          data-test="delete-agent-cancel"
          @click="close"
        >
          ×
        </button>
      </header>
      <p class="delete-modal__warning">{{ t('settings.tokens.delete_warning') }}</p>
      <ReauthFields
        v-model:current-password="reauth.currentPassword"
        v-model:code="reauth.code"
        :two-factor-enabled="twoFactorEnabled"
        variant="server-update"
      />
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
  </div>
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
  height: 32px;
  border-radius: 7px;
  font: inherit;
  font-size: 12px;
  font-weight: 600;
}
.delete-modal__close {
  width: 30px;
  color: var(--text-muted);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  font-size: 20px;
  line-height: 1;
}
.delete-modal__actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.delete-modal__cancel {
  min-width: 64px;
  color: var(--text-primary);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
}
.delete-modal__confirm {
  min-width: 96px;
  color: var(--accent-red);
  background: transparent;
  border: 1px solid currentColor;
}
.delete-modal__close:disabled,
.delete-modal__cancel:disabled,
.delete-modal__confirm:disabled {
  cursor: not-allowed;
  opacity: 0.58;
}
</style>
