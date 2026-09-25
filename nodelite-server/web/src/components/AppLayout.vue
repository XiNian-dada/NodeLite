<script setup lang="ts">
import { computed, onMounted } from 'vue';
import SidebarNav from '@/components/SidebarNav.vue';
import ConnectionStatus from '@/components/ConnectionStatus.vue';
import { useTheme } from '@/composables/useTheme';
import { useLanguage } from '@/i18n/language';
import { SUPPORTED_LOCALES, type SupportedLocale } from '@/i18n';
import { useBootstrapStore } from '@/stores/bootstrap';

// Shared chrome for every top-level view: sidebar rail + a header whose
// left side is a per-view #title slot and right side holds the global
// theme + language controls. Body goes in the default slot. Lives in one
// place so DashboardView / NodeDetailView don't each reimplement it.
const { theme, toggleTheme } = useTheme();
const { currentLocale, setLocale } = useLanguage();
const bootstrapStore = useBootstrapStore();

type GeoIpAttribution = {
  label: string;
  href?: string;
};

const geoipAttribution = computed<GeoIpAttribution | null>(() => {
  const bootstrap = bootstrapStore.data;
  if (bootstrap?.geoip_enabled && bootstrap.geoip_provider === 'dbip') {
    return {
      label: 'IP geolocation by DB-IP',
      href: 'https://db-ip.com',
    };
  }
  return null;
});

onMounted(() => {
  void bootstrapStore.load();
});

function onLanguageChange(event: Event): void {
  setLocale((event.target as HTMLSelectElement).value);
}

const localeLabels: Record<SupportedLocale, string> = {
  en: 'English',
  'zh-CN': '中文',
};
</script>

<template>
  <div class="app" data-test="app-shell">
    <SidebarNav />

    <main class="main">
      <header class="page-header">
        <div class="page-title">
          <slot name="title" />
        </div>
        <div class="page-actions">
          <select
            class="lang-select"
            :aria-label="$t('common.language')"
            data-test="language-select"
            :value="currentLocale"
            @change="onLanguageChange"
          >
            <option v-for="locale in SUPPORTED_LOCALES" :key="locale" :value="locale">
              {{ localeLabels[locale] }}
            </option>
          </select>
          <button
            type="button"
            class="theme-toggle"
            :title="$t('common.theme_toggle')"
            :aria-label="$t('common.theme_toggle')"
            data-test="theme-toggle"
            @click="toggleTheme"
          >
            <svg
              v-if="theme === 'dark'"
              class="sun"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="12" cy="12" r="4" />
              <path
                d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"
              />
            </svg>
            <svg
              v-else
              class="moon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
            </svg>
          </button>
        </div>
      </header>

      <ConnectionStatus />

      <slot />

      <footer class="app-footer" data-test="app-footer">
        <div class="footer-left">
          <span>Powered by</span>
          <a
            href="https://github.com/XiNian-dada/NodeLite"
            target="_blank"
            rel="noopener noreferrer"
            class="footer-brand"
            data-test="footer-brand-link"
          >
            <svg class="github-icon" viewBox="0 0 16 16" width="13" height="13" fill="currentColor">
              <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
            </svg>
            <span>NodeLite</span>
          </a>
        </div>

        <div v-if="geoipAttribution" class="geoip-attribution">
          <a
            v-if="geoipAttribution.href"
            :href="geoipAttribution.href"
            rel="noreferrer"
            target="_blank"
          >
            {{ geoipAttribution.label }}
          </a>
          <span v-else>{{ geoipAttribution.label }}</span>
        </div>
      </footer>
    </main>
  </div>
</template>

<style scoped>
.app {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  min-height: 100vh;
  background: var(--bg-app);
  color: var(--text-primary);
}
.main {
  padding: 24px clamp(20px, 2.5vw, 40px) 40px;
  max-width: 2560px;
  width: 100%;
}
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 24px;
  margin-bottom: 22px;
}
.page-title {
  min-width: 0;
}
.page-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}
.lang-select {
  background: var(--bg-card);
  color: var(--text-secondary);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  min-width: 104px;
  padding: 8px 12px;
  font-size: 13px;
}
.theme-toggle {
  width: 40px;
  height: 40px;
  border-radius: 8px;
  border: 1px solid var(--border-soft);
  background: var(--bg-card);
  color: var(--text-secondary);
  display: grid;
  place-items: center;
  transition:
    background 150ms ease,
    color 150ms ease;
}
.theme-toggle:hover {
  color: var(--text-primary);
  background: var(--bg-card-soft);
  border-color: var(--border-strong);
}
.theme-toggle svg {
  width: 18px;
  height: 18px;
}
.app-footer {
  margin-top: 48px;
  padding-top: 16px;
  border-top: 1px solid var(--border-soft);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px;
  color: var(--text-dim);
  font-size: 12px;
}
.footer-left {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.footer-brand {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--text-muted);
  font-weight: 500;
  text-decoration: none;
  transition: color 0.15s ease;
}
.footer-brand:hover {
  color: var(--text-primary);
}
.github-icon {
  opacity: 0.85;
}
.geoip-attribution {
  color: var(--text-dim);
  font-size: 11px;
}
.geoip-attribution a {
  color: inherit;
  text-decoration: none;
}
.geoip-attribution a:hover {
  color: var(--text-muted);
  text-decoration: underline;
}

@media (max-width: 700px) {
  .main {
    padding: 18px 12px 32px;
  }
  .page-header {
    flex-direction: column;
    gap: 12px;
  }
  .page-actions {
    align-self: stretch;
    justify-content: flex-start;
  }
}
</style>
