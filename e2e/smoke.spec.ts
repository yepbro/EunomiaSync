import { expect, test } from '@playwright/test'

test('app shell renders', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveTitle('EunomiaSync')
  const heading = page.getByRole('heading', { level: 1 })
  await expect(heading).toBeVisible()
  await expect(heading).toHaveText('EunomiaSync')
})
