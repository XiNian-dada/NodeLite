<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { apiClient } from '@/api';
import type { LastLoginInfo } from '@/api';

const show = ref(false);
const lastLogin = ref<LastLoginInfo | null>(null);
const loading = ref(true);

const DISMISSED_KEY = 'nodelite_login_notification_dismissed';

function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffHours = Math.floor(diffMs / (1000 * 60 * 60));
  const diffDays = Math.floor(diffHours / 24);

  if (diffDays > 0) {
    return `${diffDays} day${diffDays > 1 ? 's' : ''} ago`;
  } else if (diffHours > 0) {
    return `${diffHours} hour${diffHours > 1 ? 's' : ''} ago`;
  } else {
    return 'recently';
  }
}

function getLocationString(info: LastLoginInfo): string {
  const parts = [];
  if (info.city) parts.push(info.city);
  if (info.country) parts.push(info.country);
  return parts.length > 0 ? parts.join(', ') : 'Unknown location';
}

function dismiss() {
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
    }
  } catch (e) {
    console.warn('Failed to load last login info:', e);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void loadLastLogin();
});
</script>

<template>
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="show && lastLogin && lastLogin.timestamp" class="login-notification-overlay">
        <article class="login-notification panel" data-test="login-notification">
          <header class="login-notification__header">
            <div class="login-notification__icon">🔐</div>
            <h3 class="login-notification__title">Last Login</h3>
            <button
              type="button"
              class="login-notification__close"
              aria-label="Dismiss"
              @click="dismiss"
            >
              ×
            </button>
          </header>

          <div class="login-notification__body">
            <div class="login-notification__info">
              <div class="login-notification__row">
                <span class="login-notification__label">Time:</span>
                <span class="login-notification__value">
                  {{ formatTimestamp(lastLogin.timestamp) }}
                </span>
              </div>
              <div class="login-notification__row">
                <span class="login-notification__label">Location:</span>
                <span class="login-notification__value">
                  {{ getLocationString(lastLogin) }}
                </span>
              </div>
              <div v-if="lastLogin.ip_address" class="login-notification__row">
                <span class="login-notification__label">IP Address:</span>
                <span class="login-notification__value login-notification__value--mono">
                  {{ lastLogin.ip_address }}
                </span>
              </div>
            </div>

            <p class="login-notification__footer">
              If this wasn't you, please secure your account immediately.
            </p>
          </div>

          <footer class="login-notification__actions">
            <button
              type="button"
              class="btn btn--primary"
              @click="dismiss"
            >
              Got it
            </button>
          </footer>
        </article>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.login-notification-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(31, 36, 33, 0.6);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
  padding: 1rem;
}

.login-notification {
  background: #FFFFFF;
  border-radius: 12px;
  max-width: 460px;
  width: 100%;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
  border: 1px solid #E7E1D7;
  overflow: hidden;
}

.login-notification__header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 1.5rem;
  background: #FBF9F5;
  border-bottom: 1px solid #E7E1D7;
}

.login-notification__icon {
  font-size: 1.5rem;
  line-height: 1;
}

.login-notification__title {
  flex: 1;
  margin: 0;
  font-size: 1.125rem;
  font-weight: 600;
  color: #1F2421;
}

.login-notification__close {
  width: 32px;
  height: 32px;
  border: none;
  background: none;
  font-size: 1.75rem;
  line-height: 1;
  color: #5C635D;
  cursor: pointer;
  border-radius: 6px;
  transition: all 0.15s ease;
  display: flex;
  align-items: center;
  justify-content: center;
}

.login-notification__close:hover {
  background: #E7E1D7;
  color: #1F2421;
}

.login-notification__body {
  padding: 1.5rem;
}

.login-notification__info {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  margin-bottom: 1.25rem;
}

.login-notification__row {
  display: flex;
  gap: 0.5rem;
  align-items: baseline;
}

.login-notification__label {
  font-size: 0.875rem;
  font-weight: 600;
  color: #5C635D;
  min-width: 90px;
}

.login-notification__value {
  flex: 1;
  font-size: 0.95rem;
  color: #1F2421;
}

.login-notification__value--mono {
  font-family: 'Courier New', monospace;
  font-size: 0.875rem;
  color: #5C635D;
}

.login-notification__footer {
  margin: 0;
  padding: 1rem;
  background: #F2E3D6;
  border-radius: 6px;
  font-size: 0.85rem;
  color: #A94E22;
  line-height: 1.5;
}

.login-notification__actions {
  padding: 1.5rem;
  background: #FBF9F5;
  border-top: 1px solid #E7E1D7;
  display: flex;
  justify-content: flex-end;
}

.btn {
  padding: 0.625rem 1.5rem;
  border: none;
  border-radius: 999px;
  font-size: 0.9rem;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.btn--primary {
  background: #C4612F;
  color: #FFFFFF;
}

.btn--primary:hover {
  background: #A94E22;
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(196, 97, 47, 0.3);
}

.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity 0.25s ease;
}

.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}

.modal-fade-enter-active .login-notification,
.modal-fade-leave-active .login-notification {
  transition: transform 0.25s ease;
}

.modal-fade-enter-from .login-notification,
.modal-fade-leave-to .login-notification {
  transform: scale(0.95);
}
</style>
