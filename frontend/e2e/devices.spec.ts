import { expect, test } from '@playwright/test'

// every spec shares one fixture server, so each test starts from the same
// draft instead of whatever the previous one left behind
test.beforeEach(async ({ request }) => {
  await request.post('__reset')
})

/**
 * Wave 4's Devices: every device, what it can do, and why one of them is
 * missing from the Hue app.
 *
 * The table, the type tabs and the three filters are the way in; the chain
 * is the answer. A device Bifrost cannot serve reads `not mapped` and opens
 * the same detail as a mapped one. The only action on a device is identify.
 */
test.describe('devices, the inventory', () => {
  test('the table lists every device with its type and mapping', async ({ page }) => {
    await page.goto('#/devices')

    const table = page.getByTestId('device-table')
    await expect(table).toBeVisible()

    const rows = table.getByTestId('device-table-row')
    await expect(rows).toHaveCount(5)

    await expect(table).toContainText('Kitchen ceiling')
    await expect(table).toContainText('Desk plug')
    await expect(table).toContainText('Hall motion')
    await expect(page.getByTestId('devices-count')).toContainText('5 of 5')

    // an unmapped row says so, and offers the reason
    await expect(table.getByTestId('device-why').first()).toContainText('not mapped')
  })

  test('the type tabs narrow the table and say how many there are', async ({ page }) => {
    await page.goto('#/devices')

    await page.getByTestId('device-tab-light').click()
    await expect(page.getByTestId('device-table-row')).toHaveCount(2)

    await page.getByTestId('device-tab-plug').click()
    await expect(page.getByTestId('device-table-row')).toHaveCount(1)
    await expect(page.getByTestId('device-table')).toContainText('Desk plug')

    await page.getByTestId('device-tab-all').click()
    await expect(page.getByTestId('device-table-row')).toHaveCount(5)
  })

  test('the filters narrow by name and by mapping', async ({ page }) => {
    await page.goto('#/devices')

    await page.getByTestId('device-search').fill('hall')
    await expect(page.getByTestId('device-table-row')).toHaveCount(2)

    await page.getByTestId('device-search').fill('')

    await page.getByTestId('device-supported-filter').selectOption('no')
    await expect(page.getByTestId('device-table-row')).toHaveCount(3)

    await page.getByTestId('device-supported-filter').selectOption('yes')
    await expect(page.getByTestId('device-table-row')).toHaveCount(2)
  })

  test('a device that is in no room breaks the chain at the room hop', async ({ page }) => {
    await page.goto('#/devices')

    await page.getByTestId('device-table').getByRole('button', { name: 'Hall lamp' }).click()

    const detail = page.getByTestId('device-detail')
    await expect(detail).toBeVisible()

    const chain = detail.getByTestId('why-chain')
    await expect(chain).toHaveAttribute('data-broken-hop', 'room')
    await expect(detail.getByTestId('why-hop-z2m')).toHaveAttribute('data-state', 'ok')
    await expect(detail.getByTestId('why-hop-group')).toHaveAttribute('data-state', 'ok')
    await expect(detail.getByTestId('why-hop-room')).toHaveAttribute('data-state', 'broken')

    // the only action is identify
    await expect(detail.getByTestId('device-identify')).toBeVisible()
    await expect(detail.getByRole('button', { name: /on|off|brightness/i })).toHaveCount(0)
  })

  test('a device whose server is not running breaks at the z2m hop', async ({ page }) => {
    await page.goto('#/devices')

    await page.getByTestId('device-table').getByRole('button', { name: 'Hall motion' }).click()

    const detail = page.getByTestId('device-detail')
    await expect(detail.getByTestId('why-chain')).toHaveAttribute('data-broken-hop', 'z2m')
    await expect(detail.getByTestId('why-hop-z2m')).toHaveAttribute('data-state', 'broken')
    // everything after a broken hop is unknown, not broken
    await expect(detail.getByTestId('why-hop-room')).toHaveAttribute('data-state', 'unknown')
  })

  test('an unmapped row opens the same detail and says why', async ({ page }) => {
    await page.goto('#/devices')

    await page.getByTestId('device-table').getByTestId('device-why').first().click()

    const detail = page.getByTestId('device-detail')
    await expect(detail.getByTestId('device-mapping')).toHaveText('not mapped')
    await expect(detail.getByTestId('device-unsupported-reason')).toContainText('plugs')
  })

  test('the join window is opened from the Devices head', async ({ page }) => {
    await page.goto('#/devices')

    const head = page.getByTestId('devices-permit-join')
    await expect(head).toBeVisible()

    await head.getByRole('button', { name: 'Open join window' }).click()
    await expect(page.getByTestId('toast-host')).toContainText('Join window open')
  })

  test('the table becomes one card per row at 390 px', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('#/devices')

    const table = page.getByTestId('device-table')
    await expect(table).toHaveAttribute('data-layout', 'cards')

    // the header text is repeated on each cell, so the card keeps its labels
    const firstRow = table.getByTestId('device-table-row').first()
    await expect(firstRow.locator('td[data-label="Device"]')).toHaveCount(1)
    await expect(firstRow.locator('td[data-label="Hue"]')).toHaveCount(1)

    // and the page does not scroll sideways
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    )
    expect(overflow).toBe(false)
  })
})
