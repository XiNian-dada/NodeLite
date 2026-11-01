// Anti-flash + 24h check run synchronously inside the inline <script>
// in index.html, before this module is even fetched. See:
//   src/composables/useTheme.ts (setupTheme)
//   src/auth/expiry.ts (checkAuthExpiry)
import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import { router } from './router';
import { setupI18n } from './i18n';
import { setupTheme } from './composables/useTheme';
import './styles/theme.css';
import './styles/swiss.css';

setupTheme();
void (async () => {
  const app = createApp(App);
  app.use(createPinia());
  app.use(router);
  await setupI18n(app);
  app.mount('#app');
})();
