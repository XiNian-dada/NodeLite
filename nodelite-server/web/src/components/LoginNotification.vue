<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { apiClient } from '@/api';
import type { LastLoginInfo } from '@/api';

const show = ref(false);
const lastLogin = ref<LastLoginInfo | null>(null);
const { t } = useI18n();

const DISMISSED_KEY = 'nodelite_login_notification_dismissed';
const AUTO_DISMISS_MS = 5000;

let dismissTimer: number | null = null;

function clearDismissTimer(): void {
  if (dismissTimer !== null) {
    window.clearTimeout(dismissTimer);
    dismissTimer = null;
  }
}

function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffHours = Math.floor(diffMs / (1000 * 60 * 60));
  const diffDays = Math.floor(diffHours / 24);

  if (diffDays > 0) {
    return t('audit.last_login.days_ago', { count: diffDays });
  }
  if (diffHours > 0) {
    return t('audit.last_login.hours_ago', { count: diffHours });
  }
  return t('audit.last_login.recent');
}

function getLocationString(info: LastLoginInfo): string {
  const parts: string[] = [];
  if (info.city) parts.push(info.city);
  if (info.country) parts.push(info.country);
  return parts.length > 0 ? parts.join(', ') : t('audit.last_login.unknown_location');
}

function scheduleAutoDismiss(): void {
  clearDismissTimer();
  dismissTimer = window.setTimeout(() => {
    dismiss();
  }, AUTO_DISMISS_MS);
}

function dismiss(): void {
  clearDismissTimer();
  show.value = false;
  sessionStorage.setItem(DISMISSED_KEY, 'true');
}

async function loadLastLogin() {
  try {
    const info = await apiClient.lastLogin();
    lastLogin.value = info;

    // Only show if there was a previous login and user hasn't dismissed this session
    const dismissed = sessionStorage.getItem(DISMISSED_KEY);
    if (info.timestamp && !dismissed) {
      show.value = true;
      scheduleAutoDismiss();
    }
  } catch (e) {
    console.warn('Failed to load last login info:', e);
  }
}

onMounted(() => {
  void loadLastLogin();
});

onBeforeUnmount(() => {
  clearDismissTimer();
});
</script>

<template>
  <Teleport to="body">
    <Transition name="toast-slide">
      <article
        v-if="show && lastLogin && lastLogin.timestamp"
        class="login-notification panel"
        data-test="login-notification"
        role="status"
        aria-live="polite"
      >
        <div class="login-notification__accent" />
        <header class="login-notification__header">
          <div class="login-notification__badge" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <rect x="5" y="10" width="14" height="9" rx="2" />
              <path d="M8 10V7a4 4 0 1 1 8 0v3" />
            </svg>
          </div>
          <div class="login-notification__heading">
            <h3 class="login-notification__title">{{ t('audit.last_login.title') }}</h3>
            <p class="login-notification__subtitle">{{ formatTimestamp(lastLogin.timestamp) }}</p>
          </div>
          <button
            type="button"
            class="login-notification__close"
            :aria-label="t('common.close')"
            data-test="login-notification-close"
            @click="dismiss"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <path d="M6 6l12 12M18 6 6 18" />
            </svg>
          </button>
        </header>

        <div class="login-notification__body">
          <dl class="login-notification__info">
            <div class="login-notification__row">
              <dt class="login-notification__label">{{ t('audit.last_login.time') }}</dt>
              <dd class="login-notification__value">
                {{ formatTimestamp(lastLogin.timestamp) }}
              </dd>
            </div>
            <div class="login-notification__row">
              <dt class="login-notification__label">{{ t('audit.last_login.location') }}</dt>
              <dd class="login-notification__value">
                {{ getLocationString(lastLogin) }}
              </dd>
            </div>
            <div v-if="lastLogin.ip_address" class="login-notification__row">
              <dt class="login-notification__label">{{ t('audit.last_login.ip_address') }}</dt>
              <dd class="login-notification__value login-notification__value--mono">
                {{ lastLogin.ip_address }}
              </dd>
            </div>
          </dl>

          <p class="login-notification__notice">
            {{ t('audit.last_login.notice') }}
          </p>
        </div>
      </article>
    </Transition>
  </Teleport>
</template>

<style scoped>
.login-notification {
  position: fixed;
  right: max(16px, env(safe-area-inset-right));
  bottom: max(16px, env(safe-area-inset-bottom));
  z-index: 9999;
  width: min(360px, calc(100vw - 24px));
  background: var(--bg-card);
  border: 1px solid var(--border-soft);
  border-radius: 12px;
  box-shadow: var(--panel-shadow);
  overflow: hidden;
}

.login-notification__accent {
  height: 3px;
  background: linear-gradient(90deg, var(--accent-blue) 0%, var(--accent-green) 100%);
}

.login-notification__header {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 16px 16px 12px;
}

.login-notification__badge {
  width: 36px;
  height: 36px;
  border-radius: 10px;
  display: grid;
  place-items: center;
  background: var(--accent-blue-soft);
  color: var(--accent-blue);
  flex-shrink: 0;
}

.login-notification__badge svg {
  width: 18px;
  height: 18px;
}

.login-notification__heading {
  flex: 1;
  min-width: 0;
}

.login-notification__title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary);
}

.login-notification__subtitle {
  margin: 4px 0 0;
  font-size: 12px;
  color: var(--text-muted);
}

.login-notification__close {
  width: 32px;
  height: 32px;
  border: 1px solid var(--border-soft);
  background: var(--bg-card-soft);
  color: var(--text-muted);
  border-radius: 8px;
  display: grid;
  place-items: center;
  flex-shrink: 0;
  transition:
    background 150ms ease,
    color 150ms ease,
    border-color 150ms ease;
}

.login-notification__close svg {
  width: 14px;
  height: 14px;
}

.login-notification__close:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
  border-color: var(--border-strong);
}

.login-notification__body {
  padding: 0 16px 16px;
}

.login-notification__info {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin: 0 0 14px;
}

.login-notification__row {
  display: grid;
  grid-template-columns: 88px minmax(0, 1fr);
  gap: 8px;
  margin: 0;
}

.login-notification__label {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-muted);
  margin: 0;
}

.login-notification__value {
  min-width: 0;
  margin: 0;
  font-size: 13px;
  color: var(--text-secondary);
}

.login-notification__value--mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}

.login-notification__notice {
  margin: 0;
  padding: 10px 12px;
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.5;
}

.toast-slide-enter-active,
.toast-slide-leave-active {
  transition:
    opacity 180ms ease,
    transform 180ms ease;
}

.toast-slide-enter-from,
.toast-slide-leave-to {
  opacity: 0;
  transform: translateY(10px) translateX(12px);
}

@media (max-width: 640px) {
  .login-notification {
    left: 12px;
    right: 12px;
    width: auto;
  }

  .login-notification__row {
    grid-template-columns: 1fr;
    gap: 2px;
  }
}
</style>
