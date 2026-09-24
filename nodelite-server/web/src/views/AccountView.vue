<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import AppLayout from '@/components/AppLayout.vue';
import TwoFactorPanel from '@/components/TwoFactorPanel.vue';
import PasskeyPanel from '@/components/PasskeyPanel.vue';
import ChangePasswordCard from '@/components/ChangePasswordCard.vue';
import SettingsMessage from '@/components/SettingsMessage.vue';
import { AUTH_TIMESTAMP_KEY, LOGOUT_PATH } from '@/auth/expiry';
import { tokenRemaining } from '@/lib/format';
import { useSettingsStore } from '@/stores/settings';

const { t } = useI18n();
const store = useSettingsStore();

onMounted(() => {
  void store.load();
});

const auth = computed(() => store.data?.auth ?? null);

function sessionTtlText(seconds: number): string {
  const r = tokenRemaining(seconds);
  switch (r.kind) {
    case 'days_hours':
      return t('settings.duration.days_hours', { days: r.days, hours: r.hours });
    case 'minutes':
      return t('settings.duration.minutes', { minutes: r.minutes });
    default:
      return t('common.not_available');
  }
}

/** Drop the client session and bounce to reauth (same as the legacy logout). */
function logout(): void {
  try {
    window.localStorage.removeItem(AUTH_TIMESTAMP_KEY);
  } catch {
    /* localStorage unavailable */
  }
  window.location.assign(LOGOUT_PATH);
}
</script>

<template>
  <AppLayout>
    <template #title>
      <h1 class="page-heading">{{ t('account.heading') }}</h1>
      <p class="page-subtitle">{{ t('account.subtitle') }}</p>
    </template>

    <section class="account" data-test="account-view">
      <template v-if="auth">
        <div class="account__grid">
          <article class="panel security-card" data-test="security-card">
            <h2 class="card-title">{{ t('settings.security.title') }}</h2>
            <dl class="kv">
              <div class="kv__row">
                <dt>{{ t('settings.security.auth') }}</dt>
                <dd>{{ auth.enabled ? t('common.online') : t('common.offline') }}</dd>
              </div>
              <div class="kv__row">
                <dt>{{ t('settings.security.username') }}</dt>
                <dd>{{ auth.username || t('common.not_available') }}</dd>
              </div>
              <div class="kv__row">
                <dt>{{ t('settings.security.2fa') }}</dt>
                <dd>{{ auth.two_factor_enabled ? t('settings.enabled') : t('settings.disabled') }}</dd>
              </div>
              <div class="kv__row">
                <dt>{{ t('settings.security.passkeys') }}</dt>
                <dd>{{ auth.passkeys.length }}</dd>
              </div>
              <div class="kv__row">
                <dt>{{ t('settings.security.session_ttl') }}</dt>
                <dd>{{ sessionTtlText(auth.session_ttl_secs) }}</dd>
              </div>
            </dl>
            <div class="actions">
              <button type="button" class="btn btn--danger" data-test="account-logout" @click="logout">
                {{ t('settings.security.logout') }}
              </button>
            </div>
          </article>

          <div class="account__stack">
            <TwoFactorPanel :auth="auth" @changed="store.load()" />
            <PasskeyPanel :auth="auth" @changed="store.load()" />
            <ChangePasswordCard />
          </div>
        </div>
      </template>

      <SettingsMessage
        v-else-if="store.error"
        state="error"
        :text="store.error.message"
        data-test="account-error"
      />
      <p v-else class="placeholder" data-test="account-loading">
        {{ t('common.waiting_for_data') }}
      </p>
    </section>
  </AppLayout>
</template>

<style scoped>
.account {
  width: 100%;
  max-width: 1180px;
}
.account__grid {
  display: grid;
  grid-template-columns: minmax(280px, 0.9fr) minmax(420px, 1.35fr);
  gap: 16px;
  align-items: start;
}
.account__stack {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 16px;
}
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 16px;
  padding: 18px 20px;
}
.security-card {
  min-width: 0;
}
.card-title {
  margin: 0 0 12px;
  font-size: 14px;
  font-weight: 600;
}
.kv {
  margin: 0 0 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.kv__row {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  font-size: 13px;
}
.kv__row dt {
  color: var(--text-muted);
}
.kv__row dd {
  margin: 0;
  color: var(--text-primary);
}
.actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--actions-gap, 10px);
}
.btn {
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  border: 1px solid var(--border-soft);
  border-radius: var(--btn-radius, 8px);
  padding: var(--btn-padding, 6px 14px);
  font: inherit;
}
.btn--danger {
  color: var(--accent-red);
  border-color: var(--accent-red-soft);
  background: var(--accent-red-soft);
}
.page-heading {
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  letter-spacing: -0.01em;
}
.page-subtitle {
  margin: 4px 0 0;
  color: var(--text-muted);
  font-size: 13px;
}
.placeholder {
  color: var(--text-muted);
  font-size: 13px;
}
@media (max-width: 880px) {
  .account {
    max-width: none;
  }
  .account__grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
