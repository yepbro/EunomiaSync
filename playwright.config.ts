import { defineConfig, devices } from '@playwright/test'
import process from 'node:process'

export default defineConfig({
  testDir: 'e2e',
  use: {
    baseURL: 'http://localhost:1420',
  },
  webServer: {
    command: 'npm run dev -- --port 1420 --strictPort',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})
