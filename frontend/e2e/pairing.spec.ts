import { expect, test } from '@playwright/test'

import type { Page } from '@playwright/test'

// every spec shares one fixture server, so each test starts from the same
// pairing window and history instead of whatever the previous one left
test.beforeEach(async ({ request }) => {
  await request.post('__reset')
})

/**
 * Wave 6's Pairing screen.
 *
 * The mode is always on screen, the button opens a window that counts down,
 * the history shows what asked to pair, and the banner stays pinned at the
 * phone width the person pressing the button actually holds.
 */
async function openPairing(page: Page): Promise<void> {
  await page.goto('')
  await page.getByTestId('nav-pairing').click()
  await expect(page.getByTestId('pairing-screen')).toBeVisible()
  await expect(page.getByTestId('link-button-card')).toBeVisible()
}

test.describe('pairing', () => {
  test('shows the mode and the four client instructions', async ({ page }) => {
    await openPairing(page)

    // the mode readout is always visible, and the default install is compat
    await expect(page.getByTestId('pairing-mode')).toHaveText('compat')
    await expect(page.getByTestId('pairing-mode-explanation')).toContainText('pair at any time')

    for (const key of ['hue-app', 'sync-desktop', 'sync-box', 'tv-ambilight']) {
      await expect(page.getByTestId(`pairing-instruction-${key}`)).toBeVisible()
    }

    // the screen says plainly that the window is not access control
    await expect(page.getByTestId('link-button-honesty')).toContainText('grants nothing')
  })

  test('the button opens a window that counts down and closes', async ({ page }) => {
    await page.clock.install()
    await openPairing(page)

    await expect(page.getByTestId('pairing-banner')).toBeHidden()

    await page.getByTestId('link-button-press').click()

    await expect(page.getByTestId('pairing-banner')).toBeVisible()
    await expect(page.getByTestId('pairing-banner')).toContainText('30s left')
    await expect(page.getByTestId('link-button-state')).toHaveText('window open')

    // the countdown is driven by the clock, not by a wait
    await page.clock.runFor(5000)
    await expect(page.getByTestId('pairing-banner')).toContainText('25s left')

    await page.clock.runFor(30000)
    await expect(page.getByTestId('pairing-banner')).toBeHidden()
    await expect(page.getByTestId('link-button-state')).toHaveText('closed')
  })

  test('history shows the device type, the time and the result', async ({ page }) => {
    await openPairing(page)

    const table = page.getByTestId('pairing-history-table')
    await expect(table).toBeVisible()

    // the accepted row and the refused one, both with their device type
    await expect(page.getByTestId('pairing-history-accepted')).toHaveCount(1)
    await expect(page.getByTestId('pairing-history-refused')).toHaveCount(1)
    await expect(table).toContainText('Hue Sync Box')
    await expect(table).toContainText('hue app#2.0.0#iPhone14,3')

    // a log, not an access list: the screen says so
    await expect(page.getByTestId('pairing-history')).toContainText('cannot be revoked')
  })

  test('the banner stays pinned at 390 px', async ({ page }) => {
    await openPairing(page)

    await page.getByTestId('link-button-press').click()
    const banner = page.getByTestId('pairing-banner')
    await expect(banner).toBeVisible()

    // the person pressing the button is holding a phone: the banner is at
    // the top of the viewport, not below the fold
    const box = await banner.boundingBox()
    expect(box).not.toBeNull()
    expect(box!.y).toBeLessThan(80)

    await page.getByTestId('pairing-history-table').scrollIntoViewIfNeeded()
    const afterScroll = await banner.boundingBox()
    expect(afterScroll).not.toBeNull()
    expect(afterScroll!.y).toBeLessThan(80)
  })
})
