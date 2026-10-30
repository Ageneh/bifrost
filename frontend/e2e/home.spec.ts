import { expect, test } from '@playwright/test'

/**
 * Wave 3's Home: the doctor.
 *
 * The order is the contract — critical rows, then warnings, then exactly one
 * resolved row — and each row has to open the cause and the fix. A healthy
 * bridge collapses to a single row. No KPI tiles, no greeting.
 */
test.describe('home, the doctor', () => {
  test('leads with the failure, then the warning, then one resolved row', async ({ page }) => {
    await page.goto('')

    const rows = page.locator('[data-attention]')
    await expect(rows).toHaveCount(3)

    await expect(rows.nth(0)).toHaveAttribute('data-severity', 'crit')
    await expect(rows.nth(0)).toContainText('garage is not connected')
    await expect(rows.nth(0)).toContainText('Failed')

    await expect(rows.nth(1)).toHaveAttribute('data-severity', 'warn')
    await expect(rows.nth(1)).toContainText('1 change waiting to be applied')
    await expect(rows.nth(1)).toContainText('Warning')

    await expect(rows.nth(2)).toHaveAttribute('data-severity', 'ok')
    await expect(rows.nth(2)).toContainText('Everything else is healthy')
    await expect(rows.nth(2)).toContainText('Resolved')

    await expect(page.getByTestId('home-attention-panel')).toContainText('2 things need attention')
  })

  test('a row opens its cause and fix, with the action that fixes it', async ({ page }) => {
    await page.goto('')

    const first = page.locator('[data-attention]').first()
    const toggle = first.getByRole('button', { name: /garage is not connected/ })
    await expect(toggle).toHaveAttribute('aria-expanded', 'false')

    await toggle.click()
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')

    const card = first.locator('[role="region"]')
    await expect(card).toBeVisible()
    await expect(card).toContainText('Cause')
    await expect(card).toContainText('state Failed')
    await expect(card).toContainText('Fix')
    await expect(card.getByRole('button', { name: 'Restart service' })).toBeVisible()

    // closing it again leaves the list as it was
    await toggle.click()
    await expect(card).toBeHidden()
  })

  test('the health strip reports the bridge in a fixed order', async ({ page }) => {
    await page.goto('')

    await expect(page.getByTestId('home-health-bridge')).toContainText('reachable')
    await expect(page.getByTestId('home-health-services')).toContainText('2 of 3 running')
    await expect(page.getByTestId('home-health-z2m')).toContainText('1 of 2 connected')
    await expect(page.getByTestId('home-health-draft')).toContainText('1 change')
    await expect(page.getByTestId('home-health-restart')).toContainText('not required')
    await expect(page.getByTestId('home-health-revision')).toContainText('3')
  })

  test('quick actions are at most three and none of them is decorative', async ({ page }) => {
    await page.goto('')

    const actions = page.getByTestId('home-quick-actions').getByRole('button')
    await expect(actions).toHaveCount(3)
    await expect(page.getByTestId('quick-apply')).toBeVisible()
    await expect(page.getByTestId('quick-review')).toBeVisible()
    await expect(page.getByTestId('quick-bridge')).toBeVisible()
  })

  test('the decorative KPI tiles are gone', async ({ page }) => {
    await page.goto('')

    await expect(page.getByTestId('home-view')).toBeVisible()
    await expect(page.locator('[data-testid^="stat-"]')).toHaveCount(0)
  })

  test('a healthy bridge is one row that says so', async ({ page }) => {
    // every service running, no draft: the only row left is the resolved one
    await page.route('**/bifrost/service', route =>
      route.fulfill({
        json: {
          services: {
            a: { id: 'a', name: 'http', state: 'Running' },
            b: { id: 'b', name: 'z2m@office', state: 'Running' },
            c: { id: 'c', name: 'z2m@garage', state: 'Running' },
          },
        },
      }))
    await page.route('**/bifrost/config/draft', route =>
      route.fulfill({ json: { revision: 4, updated_at: null, config: null, validation: [] } }))
    await page.route('**/bifrost/config/diff', route => route.fulfill({ json: [] }))

    await page.goto('')

    const rows = page.locator('[data-attention]')
    await expect(rows).toHaveCount(1)
    await expect(rows.first()).toContainText('Everything is healthy')
    await expect(rows.first()).toHaveAttribute('data-severity', 'ok')
    await expect(page.getByTestId('home-attention-panel')).toContainText('Nothing needs attention')
    await expect(page.getByTestId('home-health-services')).toContainText('3 of 3 running')
    await expect(page.getByTestId('home-health-draft')).toContainText('no changes')
  })

  test('fits 390 px without sideways scrolling', async ({ page }) => {
    await page.goto('')
    await expect(page.getByTestId('home-view')).toBeVisible()

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    )
    expect(overflow).toBeLessThanOrEqual(0)

    // the strip is a two-column grid on a phone, not six readouts in a row
    const display = await page
      .getByTestId('home-health')
      .evaluate(element => getComputedStyle(element).display)
    expect(display).toBe('grid')
  })
})
