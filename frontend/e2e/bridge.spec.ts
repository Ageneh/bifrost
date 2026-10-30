import { expect, test } from '@playwright/test'

import type { Page } from '@playwright/test'

// every spec shares one fixture server, so each test starts from the same
// draft instead of whatever the previous one left behind
test.beforeEach(async ({ request }) => {
  await request.post('__reset')
})

/**
 * Wave 3's System · Bridge and network screen.
 *
 * Every field is present, each one names its blast radius, the MAC cannot be
 * edited, and a change reaches the draft as one merge patch — nothing is
 * written to the running bridge from this screen.
 */
const FIELDS = [
  'name',
  'mac',
  'ipaddress',
  'netmask',
  'gateway',
  'timezone',
  'http-port',
  'https-port',
  'entm-port',
]

async function openBridge(page: Page): Promise<void> {
  await page.goto('')
  await page.getByTestId('nav-system').click()
  await expect(page.getByTestId('bridge-screen')).toBeVisible()
  await expect(page.getByTestId('bridge-form')).toBeVisible()
}

test.describe('system · bridge and network', () => {
  test('shows every bridge field, each with its blast radius', async ({ page }) => {
    await openBridge(page)

    for (const field of FIELDS) {
      await expect(page.getByTestId(`config-field-bridge-${field}`)).toBeVisible()
    }

    await expect(page.getByTestId('config-field-bridge-name-blast')).toHaveText('· hot')
    await expect(page.getByTestId('config-field-bridge-timezone-blast')).toHaveText('· hot')
    await expect(page.getByTestId('config-field-bridge-ipaddress-blast')).toHaveText('· restart')
    await expect(page.getByTestId('config-field-bridge-http-port-blast')).toHaveText('· restart')
    await expect(page.getByTestId('config-field-bridge-mac-blast')).toHaveText('· restart required')

    // System is a live mode now; Devices and Rooms have had their screens
    // since waves 4 and 5, and the modes still without one say so
    await expect(page.getByTestId('nav-system')).toHaveAttribute('data-state', 'active')
    await expect(page.getByTestId('nav-devices')).toHaveAttribute('data-state', 'idle')
    await expect(page.getByTestId('nav-rooms')).toHaveAttribute('data-state', 'idle')
    await expect(page.getByTestId('nav-entertainment')).toHaveAttribute('data-state', 'pending')

    // the tab set keeps the screens this wave does not build visible, disabled
    await expect(page.getByTestId('system-tabs')).toBeVisible()
    await expect(page.getByTestId('system-tab-bridge')).toHaveAttribute('data-state', 'active')
    await expect(page.getByTestId('system-tab-z2m')).toHaveAttribute('data-state', 'pending')
  })

  test('the MAC is read-only: changing it would re-pair every client', async ({ page }) => {
    await openBridge(page)

    const mac = page.getByTestId('config-field-bridge-mac').locator('input')
    await expect(mac).toBeDisabled()
    await expect(mac).toHaveValue('00:11:22:33:44:55')
  })

  test('a changed port is written to the draft, not to the bridge', async ({ page }) => {
    await openBridge(page)

    const save = page.getByTestId('bridge-save')
    await expect(save).toBeDisabled()

    await page.getByTestId('config-field-bridge-http-port').locator('input').fill('8080')
    await expect(save).toBeEnabled()

    // the page rides the listener it is about to change, so it says so first
    const lockout = page.getByTestId('bridge-lockout')
    await expect(lockout).toBeVisible()
    await expect(lockout).toContainText('10.0.0.2:8080')

    await save.click()
    await expect(page.getByTestId('toast-ok')).toContainText('Saved to the draft')

    // the draft is the new baseline, so the form settles and the button locks
    await expect(save).toBeDisabled()
    await expect(page.getByTestId('config-field-bridge-http-port').locator('input')).toHaveValue('8080')
  })

  test('revert puts every field back', async ({ page }) => {
    await openBridge(page)

    const name = page.getByTestId('config-field-bridge-name').locator('input')
    await name.fill('Wohnzimmer')
    await expect(page.getByTestId('bridge-save')).toBeEnabled()

    await page.getByTestId('bridge-revert').click()
    await expect(name).toHaveValue('Bifrost')
    await expect(page.getByTestId('bridge-save')).toBeDisabled()
  })

  test('an invalid address is caught before it is saved', async ({ page }) => {
    await openBridge(page)

    await page.getByTestId('config-field-bridge-ipaddress').locator('input').fill('10.0.0')
    await expect(page.getByTestId('config-field-bridge-ipaddress')).toHaveAttribute(
      'data-state',
      'invalid',
    )
  })

  test('stacks to one column at 600 px and keeps the dark theme', async ({ page }) => {
    await page.setViewportSize({ width: 600, height: 900 })
    await openBridge(page)

    const columns = await page
      .getByTestId('bridge-form')
      .locator('.grid')
      .evaluate(element => getComputedStyle(element).gridTemplateColumns.split(' ').length)
    expect(columns).toBe(1)

    const background = await page.evaluate(() => getComputedStyle(document.body).backgroundColor)
    expect(background).toBe('rgb(8, 9, 11)')

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    )
    expect(overflow).toBeLessThanOrEqual(0)
  })
})
