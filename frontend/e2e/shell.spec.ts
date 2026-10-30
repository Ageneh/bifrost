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

  test('turns the rail into a bottom tab bar on a phone', async ({ page }) => {
    await page.goto('')

    const rail = page.getByTestId('shell-rail')
    await expect(rail).toBeVisible()

    const layout = await rail.evaluate((element) => {
      const style = getComputedStyle(element)
      return { position: style.position, bottom: style.bottom }
    })
    expect(layout.position).toBe('fixed')
    expect(layout.bottom).toBe('0px')

    // a thumb reaches the bottom of a phone and not the top
    const nav = page.getByTestId('side-nav')
    const direction = await nav.evaluate(element => getComputedStyle(element).flexDirection)
    expect(direction).toBe('row')
  })
})
