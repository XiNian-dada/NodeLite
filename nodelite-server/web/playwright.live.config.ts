import { defineConfig, devices } from '@playwright/test';

const baseURL = process.env.NODELITE_E2E_BASE_URL;
const username = process.env.NODELITE_E2E_USER;
const password = process.env.NODELITE_E2E_PASS;
if (!baseURL || !username || !password) {
  throw new Error('Run pnpm e2e:live to start the isolated Rust server and temporary credentials');
}

export default defineConfig({
  testDir: './e2e',
  testMatch: ['ws-dashboard.spec.ts', 'ws-reconnect.spec.ts'],
  outputDir: 'test-results/live',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  timeout: 45_000,
  expect: { timeout: 10_000 },
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/live', open: 'never' }],
    ['json', { outputFile: 'test-results/live-results.json' }],
  ],
  use: {
    baseURL,
    httpCredentials: { username, password, send: 'always' },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  projects: [{ name: 'chromium-live', use: { ...devices['Desktop Chrome'] } }],
});
