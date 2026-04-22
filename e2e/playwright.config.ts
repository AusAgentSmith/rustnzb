import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 30000,
  expect: { timeout: 10000 },
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  globalSetup: './global-setup.ts',
  globalTeardown: './global-teardown.ts',

  projects: [
    // ── Main: authenticated, seeded data ──────────────────────────────────────
    {
      name: 'main',
      testIgnore: ['**/first-boot.spec.ts', '**/auth.spec.ts'],
      use: {
        ...devices['Desktop Chrome'],
        baseURL: 'http://localhost:9190',
        storageState: 'auth-state.json',
        trace: 'on-first-retry',
        screenshot: 'only-on-failure',
      },
    },
    // ── Fresh/first-boot: runs first; expects no credentials on startup ──────
    // Split into two projects so auth.spec.ts waits for first-boot.spec.ts to
    // finish (and create credentials) before running. Playwright does not
    // guarantee file-alphabetical ordering otherwise — `auth` comes before
    // `first-boot` alphabetically, which would break 10.2+.
    {
      name: 'fresh',
      testMatch: ['**/first-boot.spec.ts'],
      use: {
        ...devices['Desktop Chrome'],
        baseURL: 'http://localhost:9191',
        // No storageState — tests manage their own tokens
        trace: 'on-first-retry',
        screenshot: 'only-on-failure',
      },
    },
    {
      name: 'fresh-auth',
      testMatch: ['**/auth.spec.ts'],
      dependencies: ['fresh'],
      use: {
        ...devices['Desktop Chrome'],
        baseURL: 'http://localhost:9191',
        trace: 'on-first-retry',
        screenshot: 'only-on-failure',
      },
    },
  ],
});
