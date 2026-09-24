<script setup lang="ts">
import { onMounted, ref, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient, type SettingsAuth } from '@/api';
import { authenticatePasskey, supportsPasskeys } from '@/auth/passkey';
import { finishStepUp } from '@/auth/stepUp';
import { messageFromError } from '@/lib/apiError';
import NativeDialog from './NativeDialog.vue';

const { t } = useI18n();
const titleId = useId();
const auth = ref<SettingsAuth | null>(null);
const code = ref('');
const password = ref('');
const busy = ref(false);
const error = ref('');

onMounted(async () => {
  try {
    auth.value = (await apiClient.settings()).auth;
  } catch (cause) {
    error.value = messageFromError(cause, 'unknown');
  }
});

function cancel(): void {
  if (!busy.value) finishStepUp(false);
}

async function confirmCode(): Promise<void> {
  busy.value = true;
  error.value = '';
  try {
    await apiClient.confirmSettings(
      auth.value?.two_factor_enabled ? { code: code.value } : { current_password: password.value },
    );
    finishStepUp(true);
  } catch (cause) {
    error.value = messageFromError(cause, 'unknown');
  } finally {
    busy.value = false;
  }
}

async function confirmPasskey(): Promise<void> {
  busy.value = true;
  error.value = '';
  try {
    const options = await apiClient.confirmSettingsPasskeyStart();
    const credential = await authenticatePasskey(options);
    await apiClient.confirmSettingsPasskeyFinish(credential);
    finishStepUp(true);
  } catch (cause) {
    error.value = messageFromError(cause, 'unknown');
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <NativeDialog class="step-up" :labelled-by="titleId" :dismissible="!busy" @close="cancel">
    <section class="step-up__panel" data-test="step-up-dialog">
      <h2 :id="titleId">{{ t('settings.confirm.title') }}</h2>
      <p>{{ t('settings.confirm.note') }}</p>
      <form v-if="auth" @submit.prevent="confirmCode">
        <label>
          <span>{{
            auth.two_factor_enabled
              ? t('settings.security.verification_code')
              : t('settings.password.current')
          }}</span>
          <input
            v-if="auth.two_factor_enabled"
            v-model="code"
            type="text"
            inputmode="numeric"
            pattern="[0-9]{6}"
            maxlength="6"
            autocomplete="one-time-code"
            required
            autofocus
            data-test="step-up-code"
          />
          <input
            v-else
            v-model="password"
            type="password"
            autocomplete="current-password"
            required
            autofocus
            data-test="step-up-password"
          />
        </label>
        <button type="submit" class="btn btn--primary" :disabled="busy" data-test="step-up-confirm">
          {{ t('settings.confirm.confirm') }}
        </button>
      </form>
      <div
        v-if="auth?.two_factor_enabled && auth.passkeys.length && supportsPasskeys()"
        class="step-up__passkey"
      >
        <span>{{ t('settings.confirm.or') }}</span>
        <button
          type="button"
          class="btn"
          :disabled="busy"
          data-test="step-up-passkey"
          @click="confirmPasskey"
        >
          {{ t('settings.confirm.passkey') }}
        </button>
      </div>
      <p v-if="error" class="step-up__error" role="alert">{{ error }}</p>
      <button type="button" class="btn" :disabled="busy" @click="cancel">
        {{ t('settings.passkeys.cancel') }}
      </button>
    </section>
  </NativeDialog>
</template>

<style scoped>
.step-up {
  position: fixed;
  inset: 0;
  z-index: 100;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.72);
}
.step-up__panel {
  width: min(420px, 100%);
  padding: 22px;
  border-radius: 12px;
  border: 1px solid var(--border-soft);
  background: var(--bg-card);
  box-shadow: var(--panel-shadow);
}
.step-up__panel h2 {
  margin: 0 0 8px;
  font-size: 18px;
}
.step-up__panel p {
  color: var(--text-muted);
}
.step-up__panel form,
.step-up__panel label {
  display: grid;
  gap: 10px;
}
.step-up__panel input {
  width: 100%;
  box-sizing: border-box;
  padding: 8px;
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: 6px;
  color: var(--text-primary);
}
.step-up__passkey {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 14px 0;
}
.step-up__error {
  color: var(--danger, #e66767) !important;
}
</style>
