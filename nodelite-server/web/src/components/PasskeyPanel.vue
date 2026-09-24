<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { SettingsAuth } from '@/api';
import { apiClient } from '@/api';
import { ApiAbortError } from '@/api/client';
import { createPasskey, supportsPasskeys } from '@/auth/passkey';
import { messageFromError } from '@/lib/apiError';
import SettingsMessage from './SettingsMessage.vue';

const props = defineProps<{ auth: SettingsAuth }>();
const emit = defineEmits<{ changed: [] }>();
const { t, locale } = useI18n();

const adding = ref(false);
const busy = ref(false);
const deletingId = ref<string | null>(null);
const label = ref('');
const message = reactive<{ state: 'ok' | 'error' | null; text: string }>({ state: null, text: '' });

const enabled = computed(() => props.auth.two_factor_enabled);
const browserSupported = computed(supportsPasskeys);

function resetForm(): void {
  label.value = '';
}

function showAddForm(): void {
  adding.value = true;
  deletingId.value = null;
  message.state = null;
  message.text = '';
}

function cancel(): void {
  adding.value = false;
  deletingId.value = null;
  resetForm();
  message.state = null;
  message.text = '';
}

function formattedDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium' }).format(date);
}

async function register(): Promise<void> {
  busy.value = true;
  message.state = null;
  message.text = t('settings.passkeys.adding');
  try {
    const options = await apiClient.passkeyRegistrationStart({
      label: label.value,
    });
    const credential = await createPasskey(options);
    await apiClient.passkeyRegistrationFinish(credential);
    adding.value = false;
    resetForm();
    message.state = 'ok';
    message.text = t('settings.passkeys.added');
    emit('changed');
  } catch (error) {
    if (error instanceof ApiAbortError) {
      message.text = '';
      return;
    }
    message.state = 'error';
    message.text = t('settings.passkeys.action_failed', {
      error: messageFromError(error, 'unknown'),
    });
  } finally {
    busy.value = false;
  }
}

function requestDelete(id: string): void {
  deletingId.value = id;
  adding.value = false;
  message.state = null;
  message.text = '';
}

async function remove(): Promise<void> {
  if (!deletingId.value) return;
  busy.value = true;
  message.state = null;
  message.text = t('settings.passkeys.removing');
  try {
    await apiClient.deletePasskey(deletingId.value, {});
    deletingId.value = null;
    message.state = 'ok';
    message.text = t('settings.passkeys.removed');
    emit('changed');
  } catch (error) {
    if (error instanceof ApiAbortError) {
      message.text = '';
      return;
    }
    message.state = 'error';
    message.text = t('settings.passkeys.action_failed', {
      error: messageFromError(error, 'unknown'),
    });
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <article class="panel" data-test="passkey-panel">
    <h2 class="card-title">{{ t('settings.passkeys.title') }}</h2>
    <p v-if="!enabled" class="note">{{ t('settings.passkeys.requires_2fa') }}</p>
    <p v-else-if="!browserSupported" class="note">
      {{ t('settings.passkeys.unsupported_browser') }}
    </p>
    <template v-else>
      <p class="note">{{ t('settings.passkeys.note') }}</p>
      <ul v-if="auth.passkeys.length" class="passkey-list" data-test="passkey-list">
        <li v-for="passkey in auth.passkeys" :key="passkey.id" class="passkey-row">
          <div>
            <strong>{{ passkey.label }}</strong>
            <small>{{
              t('settings.passkeys.added_on', { date: formattedDate(passkey.created_at) })
            }}</small>
          </div>
          <button
            type="button"
            class="btn btn--danger"
            :disabled="busy"
            :data-test="`remove-passkey-${passkey.id}`"
            @click="requestDelete(passkey.id)"
          >
            {{ t('settings.passkeys.remove') }}
          </button>
        </li>
      </ul>
      <p v-else class="empty" data-test="passkey-empty">{{ t('settings.passkeys.empty') }}</p>

      <form v-if="adding" class="form" data-test="add-passkey-form" @submit.prevent="register">
        <label class="field">
          <span>{{ t('settings.passkeys.label') }}</span>
          <input
            v-model="label"
            type="text"
            maxlength="64"
            required
            :placeholder="t('settings.passkeys.label_placeholder')"
          />
        </label>
        <div class="actions">
          <button
            type="submit"
            class="btn btn--primary"
            :disabled="busy"
            data-test="confirm-add-passkey"
          >
            {{ t('settings.passkeys.add') }}
          </button>
          <button type="button" class="btn" :disabled="busy" @click="cancel">
            {{ t('settings.passkeys.cancel') }}
          </button>
        </div>
      </form>

      <form
        v-else-if="deletingId"
        class="form"
        data-test="remove-passkey-form"
        @submit.prevent="remove"
      >
        <p class="note">{{ t('settings.passkeys.remove_note') }}</p>
        <div class="actions">
          <button
            type="submit"
            class="btn btn--danger"
            :disabled="busy"
            data-test="confirm-remove-passkey"
          >
            {{ t('settings.passkeys.remove') }}
          </button>
          <button type="button" class="btn" :disabled="busy" @click="cancel">
            {{ t('settings.passkeys.cancel') }}
          </button>
        </div>
      </form>

      <button
        v-else
        type="button"
        class="btn btn--primary"
        :disabled="busy"
        data-test="add-passkey"
        @click="showAddForm"
      >
        {{ t('settings.passkeys.add') }}
      </button>
    </template>
    <SettingsMessage :state="message.state" :text="message.text" />
  </article>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 18px 20px;
}
.card-title {
  margin: 0 0 12px;
  font-size: 14px;
  font-weight: 600;
}
.note,
.empty {
  margin: 0 0 12px;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}
.passkey-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0 0 12px;
  padding: 0;
  list-style: none;
}
.passkey-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px;
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  background: var(--bg-card-soft);
}
.passkey-row strong,
.passkey-row small {
  display: block;
}
.passkey-row small {
  margin-top: 3px;
  color: var(--text-muted);
  font-size: 12px;
}
.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  color: var(--text-muted);
  font-size: 13px;
}
.field input {
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 8px 10px;
  color: var(--text-primary);
  background: var(--bg-card-soft);
  font: inherit;
}
.actions {
  display: flex;
  align-items: center;
  gap: var(--actions-gap, 10px);
}
.btn {
  align-self: flex-start;
  border: 1px solid var(--border-soft);
  border-radius: var(--btn-radius, 8px);
  padding: var(--btn-padding, 6px 14px);
  color: var(--text-secondary);
  background: var(--bg-card-soft);
  font: inherit;
}
.btn--primary {
  border-color: transparent;
  color: #fff;
  background: var(--accent-blue);
}
.btn--danger {
  border-color: var(--accent-red-soft);
  color: var(--accent-red);
  background: var(--accent-red-soft);
}
</style>
