import { defineConfig, devices } from '@playwright/test'

// Browser tests (see CONTRIBUTING.md, "Browser tests"). Two layers:
//   mock/  the panel UI built with the mock backend (VITE_MOCK=1), served by `vite preview`: fast, deterministic.
//   real/  a smoke layer against the real `kora panel` binary (started by e2e/real/global-setup.ts). Run with
//          `npm run e2e:real`; it needs a built `kora` binary (KORA_BIN, default ../target/debug/kora).
const PORT = Number(process.env.E2E_MOCK_PORT ?? 4173)
const CI = !!process.env.CI

export default defineConfig({
  testDir: './e2e',
  outputDir: './e2e-results',
  fullyParallel: true,
  forbidOnly: CI,
  retries: CI ? 1 : 0,
  workers: CI ? 2 : undefined,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: CI ? [['list'], ['html', { open: 'never', outputFolder: 'e2e-report' }]] : [['list']],
  use: {
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off',
    locale: 'en-GB',
    timezoneId: 'Europe/London',
  },
  projects: [
    {
      name: 'mock-chromium',
      testDir: './e2e/mock',
      use: { ...devices['Desktop Chrome'], baseURL: `http://127.0.0.1:${PORT}` },
    },
    {
      // One cheap cross-engine run of the core journey. Only when asked for (CI sets E2E_CROSS_BROWSER=1).
      name: 'mock-webkit',
      testDir: './e2e/mock',
      testMatch: /core\.cross\.spec\.ts/,
      use: { ...devices['Desktop Safari'], baseURL: `http://127.0.0.1:${PORT}` },
    },
    {
      name: 'real-chromium',
      testDir: './e2e/real',
      use: { ...devices['Desktop Chrome'] },
      workers: 1,
      fullyParallel: false,
    },
  ],
  // Only the mock layer needs the preview server; `--project real-chromium` starts its own panel.
  webServer: process.env.E2E_NO_MOCK_SERVER
    ? undefined
    : {
        // A separate output folder, so the normal `dist` is not touched. VITE_MOCK=1 compiles the mock backend in.
        command: `npx vite build --outDir dist-mock --emptyOutDir && npx vite preview --outDir dist-mock --host 127.0.0.1 --port ${PORT} --strictPort`,
        url: `http://127.0.0.1:${PORT}`,
        env: { VITE_MOCK: '1' },
        reuseExistingServer: !CI,
        timeout: 180_000,
        stdout: 'ignore',
      },
})
