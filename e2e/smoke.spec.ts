import { expect, test } from '@playwright/test'

// Spec §15.1 (D-007/D-108): e2e drives the UI against the Vite dev server
// with a MOCKED IPC backend — playwright.config.ts passes VITE_IPC_MOCK to
// the server, so this suite always runs on the mock layer (T-141).
test('app shell renders', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveTitle('EunomiaSync')
  const heading = page.getByRole('heading', { level: 1 })
  await expect(heading).toBeVisible()
  await expect(heading).toHaveText('EunomiaSync')
})

test('the mock IPC backend answers app_info (T-141)', async ({ page }) => {
  await page.goto('/')
  // The version line only renders after `ipc.commands.appInfo()` resolves
  // — a real facade round-trip, which without the mock backend cannot
  // happen in a plain browser.
  const version = page.locator('.version')
  await expect(version).toBeVisible()
  await expect(version).toHaveText(/^v\d+\.\d+\.\d+/)
})
