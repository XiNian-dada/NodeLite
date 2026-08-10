<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient, type GenerateAgentInstallResponse, type SettingsResponse } from '@/api';
import { ApiAbortError } from '@/api/client';
import { messageFromError } from '@/lib/apiError';
import CsvField from './CsvField.vue';
import ReauthFields from './ReauthFields.vue';
import SettingsMessage from './SettingsMessage.vue';

const props = defineProps<{ settings: SettingsResponse }>();
const emit = defineEmits<{ created: [] }>();
const { t, locale } = useI18n();

const open = ref(false);
const submitting = ref(false);
const result = ref<GenerateAgentInstallResponse | null>(null);
const draft = reactive({ nodeId: '', nodeLabel: '', tags: [] as string[] });
const reauth = reactive({ currentPassword: '', code: '' });
const message = reactive<{ state: 'ok' | 'error' | null; text: string }>({
  state: null,
  text: '',
});
const copyMessage = reactive<{ state: 'ok' | 'error' | null; text: string }>({
  state: null,
  text: '',
});

const authEnabled = computed(() => props.settings.auth.enabled);
const twoFactorEnabled = computed(() => props.settings.auth.two_factor_enabled);

function formatDateTime(value: string): string {
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString(locale.value) : value;
}

function openForm(): void {
  open.value = true;
  message.state = null;
  message.text = '';
  copyMessage.state = null;
  copyMessage.text = '';
}

function confirmationPayload() {
  return twoFactorEnabled.value
    ? { code: reauth.code }
    : { current_password: reauth.currentPassword };
}

async function generate(): Promise<void> {
  const nodeId = draft.nodeId.trim();
  if (!nodeId) {
    message.state = 'error';
    message.text = t('settings.install.node_id_required');
    return;
  }

  submitting.value = true;
  message.state = null;
  message.text = t('settings.install.generating');
  copyMessage.state = null;
  copyMessage.text = '';
  try {
    const nodeLabel = draft.nodeLabel.trim();
    result.value = await apiClient.generateAgentInstall({
      node_id: nodeId,
      tags: draft.tags,
      ...(nodeLabel ? { node_label: nodeLabel } : {}),
      ...confirmationPayload(),
    });
    reauth.currentPassword = '';
    reauth.code = '';
    message.state = 'ok';
    message.text = result.value.message || t('settings.install.generated');
    emit('created');
  } catch (error) {
    if (error instanceof ApiAbortError) return;
    message.state = 'error';
    message.text = t('settings.install.generate_failed', {
      error: messageFromError(error, 'unknown'),
    });
  } finally {
    submitting.value = false;
  }
}

async function copyCommand(): Promise<void> {
  const command = result.value?.install_command;
  if (!command || !navigator.clipboard) {
    copyMessage.state = 'error';
    copyMessage.text = t('settings.install.copy_failed');
    return;
  }
  try {
    await navigator.clipboard.writeText(command);
    copyMessage.state = 'ok';
    copyMessage.text = t('settings.install.copied');
  } catch {
    copyMessage.state = 'error';
    copyMessage.text = t('settings.install.copy_failed');
  }
}
</script>

<template>
  <article class="panel" data-test="install-agent-card">
    <header class="card-head">
      <div>
        <span class="card-kicker">{{ t('settings.install.kicker') }}</span>
        <h2 class="card-title">{{ t('settings.install.title') }}</h2>
        <p class="card-note">{{ t('settings.install.note') }}</p>
      </div>
      <button
        class="btn btn--primary"
        type="button"
        :disabled="!authEnabled"
        data-test="open-install-agent"
        @click="openForm"
      >
        {{ t('settings.install.open') }}
      </button>
    </header>

    <p v-if="!authEnabled" class="unavailable" data-test="install-agent-unavailable">
      {{ t('settings.install.unavailable') }}
    </p>

    <form
      v-else-if="open"
      class="install-form"
      data-test="install-agent-form"
      @submit.prevent="generate"
    >
      <div class="fields">
        <label class="field">
          <span>{{ t('settings.install.node_id') }}</span>
          <input
            v-model="draft.nodeId"
            type="text"
            maxlength="128"
            autocomplete="off"
            required
            data-test="install-agent-node-id"
          />
        </label>
        <label class="field">
          <span>{{ t('settings.install.node_label') }}</span>
          <input
            v-model="draft.nodeLabel"
            type="text"
            maxlength="256"
            autocomplete="off"
            data-test="install-agent-node-label"
          />
        </label>
        <label class="field field--wide">
          <span>{{ t('settings.install.tags') }}</span>
          <CsvField
            v-model="draft.tags"
            :placeholder="t('settings.install.tags_placeholder')"
            data-test="install-agent-tags"
          />
        </label>
      </div>
      <ReauthFields
        v-model:current-password="reauth.currentPassword"
        v-model:code="reauth.code"
        :two-factor-enabled="twoFactorEnabled"
        variant="server-update"
      />
      <p class="rotate-notice">{{ t('settings.install.rotate_notice') }}</p>
      <button
        class="btn btn--primary"
        type="submit"
        :disabled="submitting"
        data-test="generate-install-agent"
      >
        {{ submitting ? t('settings.install.generating') : t('settings.install.generate') }}
      </button>
      <SettingsMessage :state="message.state" :text="message.text" />

      <section v-if="result" class="install-result" data-test="install-agent-result">
        <div class="result-head">
          <div>
            <h3>{{ t('settings.install.command_title', { node: result.node_label }) }}</h3>
            <p>
              {{
                t('settings.install.expires_at', {
                  time: formatDateTime(result.install_token_expires_at),
                })
              }}
            </p>
          </div>
          <button class="btn" type="button" data-test="copy-install-command" @click="copyCommand">
            {{ t('settings.install.copy') }}
          </button>
        </div>
        <pre class="install-command" data-test="install-agent-command">{{
          result.install_command
        }}</pre>
        <SettingsMessage :state="copyMessage.state" :text="copyMessage.text" />
      </section>
    </form>
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 16px;
}
.card-head,
.result-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}
.card-kicker {
  display: block;
  color: var(--text-muted);
  font-size: 12px;
  margin-bottom: 4px;
}
.card-title,
.result-head h3 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.card-note,
.result-head p,
.unavailable {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 12px;
}
.unavailable {
  margin-top: 14px;
}
.install-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 16px;
  padding-top: 16px;
  border-top: 1px solid var(--border-soft);
}
.fields {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  color: var(--text-muted);
  font-size: 13px;
}
.field--wide {
  grid-column: 1 / -1;
}
.field input {
  width: 100%;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  color: var(--text-primary);
  padding: 8px 10px;
  font: inherit;
}
.btn {
  align-self: flex-start;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  padding: 8px 14px;
  font: inherit;
}
.btn:hover:not([disabled]) {
  color: var(--text-primary);
}
.btn--primary {
  border-color: var(--accent-blue);
  background: var(--accent-blue);
  color: #fff;
}
.btn--primary:hover:not([disabled]) {
  filter: brightness(1.08);
}
.btn:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
.install-result {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-top: 4px;
}
.rotate-notice {
  margin: 0;
  color: var(--accent-yellow);
  font-size: 12px;
}
.install-command {
  margin: 0;
  overflow-x: auto;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  color: var(--text-primary);
  padding: 12px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 12px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-all;
}
@media (max-width: 520px) {
  .card-head,
  .result-head {
    flex-direction: column;
  }
  .fields {
    grid-template-columns: minmax(0, 1fr);
  }
  .field--wide {
    grid-column: auto;
  }
}
</style>
