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

    <div class="account-container">
      <section class="account" data-test="account-view">
        <template v-if="auth">
          <div class="account__grid">
            <aside class="account__sidebar">
              <article class="panel security-card" data-test="security-card">
                <div class="user-profile-header">
                  <div class="user-avatar">
                    <span class="user-avatar-icon">🛡️</span>
                  </div>
                  <h2 class="user-name">{{ auth.username || t('settings.security.title') }}</h2>
                  <span class="user-role-badge">
                    {{ auth.enabled ? t('common.online') : t('common.offline') }}
                  </span>
                </div>

                <div class="profile-divider" />

                <dl class="kv">
                  <div class="kv__row">
                    <dt>{{ t('settings.security.auth') }}</dt>
                    <dd class="status-indicator">
                      <span class="dot" :class="auth.enabled ? 'dot--active' : 'dot--inactive'" />
                      {{ auth.enabled ? t('common.online') : t('common.offline') }}
                    </dd>
                  </div>
                  <div class="kv__row">
                    <dt>{{ t('settings.security.username') }}</dt>
                    <dd class="font-mono">{{ auth.username || t('common.not_available') }}</dd>
                  </div>
                  <div class="kv__row">
                    <dt>{{ t('settings.security.2fa') }}</dt>
                    <dd>
                      <span class="mini-badge" :class="auth.two_factor_enabled ? 'mini-badge--success' : 'mini-badge--neutral'">
                        {{ auth.two_factor_enabled ? t('settings.enabled') : t('settings.disabled') }}
                      </span>
                    </dd>
                  </div>
                  <div class="kv__row">
                    <dt>{{ t('settings.security.passkeys') }}</dt>
                    <dd>
                      <span class="mini-badge" :class="auth.passkeys.length > 0 ? 'mini-badge--success' : 'mini-badge--neutral'">
                        {{ auth.passkeys.length }}
                      </span>
                    </dd>
                  </div>
                  <div class="kv__row">
                    <dt>{{ t('settings.security.session_ttl') }}</dt>
                    <dd class="session-ttl">{{ sessionTtlText(auth.session_ttl_secs) }}</dd>
                  </div>
                </dl>

                <div class="actions">
                  <button type="button" class="btn btn--danger btn--full" data-test="account-logout" @click="logout">
                    <span class="btn-icon">⎋</span>
                    {{ t('settings.security.logout') }}
                  </button>
                </div>
              </article>
            </aside>

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
    </div>
  </AppLayout>
</template>

<style scoped>
.account-container {
  width: 100%;
  display: flex;
  justify-content: flex-start;
}
.account {
  width: 100%;
  max-width: 1020px;
}
.account__grid {
  display: grid;
  grid-template-columns: 310px minmax(0, 1fr);
  gap: 20px;
  align-items: start;
}
.account__sidebar {
  position: sticky;
  top: 16px;
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
  border-radius: var(--radius-lg);
  padding: 20px;
}
.security-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.user-profile-header {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 6px 0 10px;
}
.user-avatar {
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: var(--bg-card-soft);
  border: 2px solid var(--border-soft);
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 12px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
}
.user-avatar-icon {
  font-size: 30px;
}
.user-name {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
}
.user-role-badge {
  display: inline-block;
  margin-top: 6px;
  font-size: 11px;
  font-weight: 500;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  background: var(--accent-green-soft);
  color: var(--accent-green);
}
.profile-divider {
  height: 1px;
  background: var(--border-soft);
  margin: 14px 0 16px;
}
.kv {
  margin: 0 0 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.kv__row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  font-size: 13px;
}
.kv__row dt {
  color: var(--text-muted);
}
.kv__row dd {
  margin: 0;
  color: var(--text-primary);
  display: flex;
  align-items: center;
}
.font-mono {
  font-family: monospace;
}
.status-indicator {
  display: flex;
  align-items: center;
  gap: 6px;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
}
.dot--active {
  background: var(--accent-green);
  box-shadow: 0 0 6px var(--accent-green);
}
.dot--inactive {
  background: var(--text-dim);
}
.mini-badge {
  font-size: 11px;
  font-weight: 500;
  padding: 1px 7px;
  border-radius: var(--radius-sm);
}
.mini-badge--success {
  background: var(--accent-green-soft);
  color: var(--accent-green);
}
.mini-badge--neutral {
  background: var(--bg-card-soft);
  color: var(--text-muted);
}
.session-ttl {
  color: var(--accent-blue);
  font-weight: 500;
  font-size: 12px;
}
.actions {
  display: flex;
  align-items: center;
  width: 100%;
}
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border-radius: var(--btn-radius, 8px);
  padding: var(--btn-padding, 8px 14px);
  font: inherit;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}
.btn--full {
  width: 100%;
}
.btn--danger {
  color: var(--accent-red);
  border: 1px solid var(--accent-red-soft);
  background: var(--accent-red-soft);
}
.btn--danger:hover {
  background: rgba(255, 77, 109, 0.25);
  border-color: var(--accent-red);
}
.btn-icon {
  font-size: 14px;
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
  .account__sidebar {
    position: static;
  }
}
</style>
