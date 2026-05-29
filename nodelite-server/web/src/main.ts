// Anti-flash + 24h check run synchronously inside the inline <script>
// in index.html, before this module is even fetched. See:
//   src/composables/useTheme.ts (setupTheme)
//   src/auth/expiry.ts (checkAuthExpiry)
import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import { router } from './router';
import { setupI18n } from './i18n';
import './styles/theme.css';

void (async () => {
  const app = createApp(App);
  app.use(createPinia());
  app.use(router);
  try {
    await setupI18n(app);
  } catch (e) {
    console.error('i18n load failed, continuing with empty dictionary', e);
  }
  app.mount('#app');
})();
