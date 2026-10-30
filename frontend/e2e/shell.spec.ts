import { expect, test } from '@playwright/test'

/**
 * Wave 2's acceptance case: the shell, the six nav items, the draft pill and
 * the card-collapsed table, at 390 px behind an ingress prefix.
 */
test.describe('the console shell at 390 px', () => {
  test('loads behind the ingress prefix and renders the shell', async ({ page }) => {
    const failedAssets: string[] = []
    page.on('response', (response) => {
      if (response.url().includes('/_nuxt/') && response.status() >= 400) {
        failedAssets.push(`${response.status()} ${response.url()}`)
      }
    })

    await page.goto('')

    await expect(page.getByTestId('app-shell')).toBeVisible()
    await expect(page.getByTestId('shell-bridge-name')).toHaveText('Bifrost')
    await expect(page.getByTestId('shell-status')).toBeVisible()

    // every asset the shell loaded came from the prefix, not from the host root
    const assetUrls = await page.evaluate(() =>
      performance
        .getEntriesByType('resource')
        .map(entry => entry.name)
        .filter(name => name.includes('_nuxt')),
    )
    expect(assetUrls.length).toBeGreaterThan(0)
    for (const url of assetUrls) {
      expect(url).toContain('/api/hassio_ingress/test/_nuxt/')
    }
    expect(failedAssets).toEqual([])
  })

  test('shows the six frozen nav items and a working route', async ({ page }) => {
    await page.goto('')

    const nav = page.getByTestId('side-nav')
    await expect(nav).toBeVisible()
    for (const key of ['home', 'devices', 'rooms', 'entertainment', 'pairing', 'system']) {
      await expect(page.getByTestId(`nav-${key}`)).toBeVisible()
    }

    // a lazy route chunk loads through the relative asset base
    await page.getByTestId('nav-devices').click()
    await expect(page.getByTestId('pending-screen')).toBeVisible()
    await expect(page.getByTestId('pending-screen')).toContainText('Devices')
  })

  test('shows the draft pill and hides the status-column bar', async ({ page }) => {
    await page.goto('')

    await expect(page.getByTestId('draft-pill')).toBeVisible()
    await expect(page.getByTestId('draft-pill-toggle')).toBeVisible()
    await expect(page.getByTestId('draft-bar')).toBeHidden()

    // tapping the pill reveals the same actions
    await page.getByTestId('draft-pill-toggle').click()
    await expect(page.getByTestId('draft-pill-apply')).toBeVisible()
  })

  test('collapses the services table into cards', async ({ page }) => {
    await page.goto('')

    const table = page.getByTestId('home-services')
    await expect(table).toHaveAttribute('data-layout', 'cards')

    // the header is gone and the column names move into each cell
    await expect(table.locator('thead')).toBeHidden()
    const firstCell = table.locator('tbody tr').first().locator('td').first()
    await expect(firstCell).toHaveAttribute('data-label', 'Service')

    // the row reads as a block, not as table cells side by side
    const display = await table.locator('tbody tr').first().evaluate(
      element => getComputedStyle(element).display,
    )
    expect(display).toBe('grid')
  })

  test('reports bridge state from the API', async ({ page }) => {
    await page.goto('')

    await expect(page.getByTestId('stat-servers')).toContainText('2')
    await expect(page.getByTestId('home-services').locator('tbody tr')).toHaveCount(3)
    // the failed service raises an attention row
    await expect(page.getByTestId('home-attention')).toContainText('z2m@garage failed')
  })
})
