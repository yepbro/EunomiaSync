import { defineConfig, devices } from '@playwright/test'
import process from 'node:process'

export default defineConfig({
  testDir: 'e2e',
  use: {
    baseURL: 'http://localhost:1420',
  },
  webServer: {
    // Run vite directly (not via `pnpm run dev`): pnpm 12.6.0 does not forward
    // SIGTERM to script children, so the spawned vite outlives the run, keeps
    // the stdio pipes and the port open, and Playwright's webServer teardown
    // never exits. Revisit on pnpm upgrades (DECISIONS.md D-018).
    command: 'node node_modules/vite/bin/vite.js --port 1420 --strictPort',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
    // Spec §15.1: e2e drives the UI against the Vite dev server with a
    // MOCKED IPC backend — the same VITE_IPC_MOCK flag browser dev and
    // Vitest use (T-141/D-108). Passed here so a plain `pnpm run e2e` (CI
    // included) is mock-backed without remembering the flag.
    env: {
      ...process.env,
      VITE_IPC_MOCK: '1',
    },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})
