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

// UI-021 (T-145): the theme follows the OS setting and switches LIVE —
// a real browser, the real compiled CSS and the real matchMedia. The
// attribute assertions tie the painted colors to the JS bridge (the CSS
// fallback alone would also paint correctly with the bridge dead); the
// unit suite owns the bridge logic, this closes the loop to pixels.
test('the theme follows the OS color scheme and switches without a reload (T-145)', async ({
  page,
}) => {
  const shell = page.locator('main')
  const html = page.locator('html')
  const backgroundOf = () => shell.evaluate((el) => getComputedStyle(el).backgroundColor)

  await page.emulateMedia({ colorScheme: 'light' })
  await page.goto('/')
  await expect(shell).toBeVisible()
  await expect(html).toHaveAttribute('data-theme', 'light')
  await expect.poll(backgroundOf).toBe('rgb(238, 242, 247)') // --page (light)

  await page.emulateMedia({ colorScheme: 'dark' })
  await expect(html).toHaveAttribute('data-theme', 'dark')
  await expect.poll(backgroundOf).toBe('rgb(11, 18, 32)') // --page (dark)

  await page.emulateMedia({ colorScheme: 'light' })
  await expect(html).toHaveAttribute('data-theme', 'light')
  await expect.poll(backgroundOf).toBe('rgb(238, 242, 247)')
})

// The pre-JS paint (T-145 round-1 P2-1): with every script blocked, a
// dark OS still paints dark — the media-query fallback in theme.css,
// no bridge involved. The html background (the :root canvas) is the
// observable: no app has mounted, so there is no <main> to read.
test('a dark OS paints dark before any JavaScript runs (CSS fallback)', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' })
  await page.route('**/*.js', (route) => route.abort())
  await page.goto('/')
  const background = await page.evaluate(
    () => getComputedStyle(document.documentElement).backgroundColor,
  )
  expect(background).toBe('rgb(11, 18, 32)') // --page (dark)
})
