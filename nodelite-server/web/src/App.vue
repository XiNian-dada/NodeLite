<script setup lang="ts">
import { onMounted } from 'vue';
import { useTheme } from '@/composables/useTheme';
import { usePolling } from '@/composables/usePolling';
import { useLanguage } from '@/i18n/language';
import { useBootstrapStore } from '@/stores/bootstrap';
import { useNodesStore } from '@/stores/nodes';
import { SUPPORTED_LOCALES } from '@/i18n';

const { theme, toggleTheme } = useTheme();
const { currentLocale, setLocale } = useLanguage();

const bootstrapStore = useBootstrapStore();
const nodesStore = useNodesStore();

onMounted(() => {
  void bootstrapStore.load();
});

// Default polling cadence matches legacy REFRESH_MS=5000. Stage 2 will
// read the real interval from the bootstrap response.
usePolling(() => nodesStore.refresh(), 5000);
</script>

<template>
  <div class="app-shell" data-test="app-shell">
    <header class="app-shell__bar">
      <strong class="app-shell__brand">NodeLite</strong>
      <div class="app-shell__controls">
        <button
          type="button"
          class="app-shell__toggle"
          data-test="theme-toggle"
          @click="toggleTheme"
        >
          {{ theme === 'dark' ? '☀︎' : '☾' }}
          <span class="app-shell__toggle-label">{{ $t('common.theme_toggle') }}</span>
        </button>
        <label class="app-shell__lang">
          <span class="app-shell__lang-label">{{ $t('common.language') }}</span>
          <select
            data-test="language-select"
            :value="currentLocale"
            @change="setLocale(($event.target as HTMLSelectElement).value)"
          >
            <option v-for="locale in SUPPORTED_LOCALES" :key="locale" :value="locale">
              {{ locale }}
            </option>
          </select>
        </label>
      </div>
    </header>
    <main class="app-shell__main">
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
.app-shell {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--bg-app);
  color: var(--text-primary);
}
.app-shell__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 24px;
  background: var(--bg-sidebar);
  border-bottom: 1px solid var(--border-soft);
}
.app-shell__brand {
  font-size: 1rem;
}
.app-shell__controls {
  display: flex;
  align-items: center;
  gap: 16px;
}
.app-shell__toggle {
  background: var(--bg-card-soft);
  color: var(--text-secondary);
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  padding: 6px 12px;
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.app-shell__toggle-label {
  font-size: 0.85rem;
}
.app-shell__lang {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--text-secondary);
  font-size: 0.85rem;
}
.app-shell__lang select {
  background: var(--bg-card-soft);
  color: var(--text-primary);
  border: 1px solid var(--border-soft);
  border-radius: 6px;
  padding: 4px 8px;
}
.app-shell__main {
  flex: 1;
  min-height: 0;
}
</style>
