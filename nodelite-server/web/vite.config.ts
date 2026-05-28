import { fileURLToPath, URL } from 'node:url';

import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

const BACKEND_URL = process.env.NODELITE_DEV_BACKEND ?? 'http://localhost:8080';
const BACKEND_WS = BACKEND_URL.replace(/^http/, 'ws');

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': { target: BACKEND_URL, changeOrigin: false, secure: false },
      '/ws': { target: BACKEND_WS, ws: true, changeOrigin: false, secure: false },
      '/assets/ui-i18n.json': { target: BACKEND_URL, changeOrigin: false, secure: false },
      '/logout-and-reauth': { target: BACKEND_URL, changeOrigin: false, secure: false },
      '/verify-2fa': { target: BACKEND_URL, changeOrigin: false, secure: false },
    },
  },
  build: {
    target: 'es2022',
    modulePreload: { polyfill: false },
    cssCodeSplit: true,
    sourcemap: true,
    rollupOptions: {
      output: {
        assetFileNames: 'assets/[name].[hash][extname]',
        chunkFileNames: 'assets/[name].[hash].js',
        entryFileNames: 'assets/[name].[hash].js',
      },
    },
  },
});
