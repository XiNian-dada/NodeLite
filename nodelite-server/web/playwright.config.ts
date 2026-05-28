import { defineConfig, devices } from '@playwright/test';

const LEGACY_BASE_URL = process.env.NODELITE_E2E_BASE_URL ?? 'http://localhost:8080';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: LEGACY_BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    httpCredentials: process.env.NODELITE_E2E_USER
      ? {
          username: process.env.NODELITE_E2E_USER,
          password: process.env.NODELITE_E2E_PASS ?? '',
        }
      : undefined,
  },
  projects: [
    {
      name: 'chromium-desktop',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'chromium-mobile',
      use: { ...devices['iPhone 13'] },
      grep: /@mobile/,
    },
  ],
});
