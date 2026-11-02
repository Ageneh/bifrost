import { expect, test } from '@playwright/test'

// every spec shares one fixture server, so each test starts from the same
// draft instead of whatever the previous one left behind
test.beforeEach(async ({ request }) => {
  await request.post('__reset')
})

/**
 * Wave 5's Rooms: what belongs where, drafted and never immediate.
 *
 * A room is a zigbee2mqtt group, so the screen lists what the groups hold,
 * drafts a new membership into the configuration, and says what applying
 * will cost. The picker is the only way to choose a device.
 */
test.describe('rooms, drafted membership', () => {
  test('the list names every room and what is in it', async ({ page }) => {
    await page.goto('#/rooms')

    const list = page.getByTestId('rooms-list')
    await expect(list).toBeVisible()
    await expect(page.getByTestId('rooms-count')).toContainText('2 rooms')

    await expect(list.getByTestId('room-row')).toHaveCount(2)
    await expect(list).toContainText('Kitchen')
    await expect(list).toContainText('Hall')
    await expect(list.getByTestId('room-row-devices').first()).toContainText('1 device')
  })

  test('a room shows its devices and recalls a scene', async ({ page }) => {
    await page.goto('#/rooms')
    await page.getByTestId('room-row').first().getByTestId('room-open').click()

    const detail = page.getByTestId('room-detail')
    await expect(detail).toBeVisible()
    await expect(detail.getByTestId('room-name')).toHaveText('Kitchen')
    await expect(detail.getByTestId('room-topic')).toContainText('group Kitchen')

    await expect(detail.getByTestId('room-member')).toHaveCount(1)
    await expect(detail.getByTestId('room-member').first()).toContainText('Kitchen ceiling')

    // the scene that is showing says so
    await expect(detail.getByTestId('room-scene')).toHaveCount(2)
    await expect(detail.getByTestId('room-scene-active')).toHaveCount(1)

    // recalling another one moves the mark
    await detail.getByTestId('room-scene').nth(1).getByTestId('room-scene-recall').click()
    await expect(page.getByTestId('toast-host')).toContainText('Scene recalled')
    await expect(detail.getByTestId('room-scene').nth(1)).toHaveAttribute('data-active', 'true')
  })

  test('the picker is the only way to choose a device, and nothing applies yet', async ({ page }) => {
    await page.goto('#/rooms')
    await page.getByTestId('room-row').first().getByTestId('room-open').click()

    await page.getByTestId('room-change-devices').click()

    const picker = page.getByTestId('room-picker')
    await expect(picker).toBeVisible()
    // the device already in the room starts ticked
    await expect(picker.getByTestId('room-picker-row')).toHaveCount(2)

    await picker.getByTestId('room-picker-row').nth(1).getByTestId('room-picker-pick').check()
    await picker.getByTestId('room-picker-confirm').click()

    await expect(page.getByTestId('toast-host')).toContainText('Membership drafted')
    // the change is in the draft, and the screen says so
    await expect(page.getByTestId('room-pending')).toContainText('not applied')
    await expect(page.getByTestId('room-member')).toHaveCount(2)
  })

  test('a rename says what it costs and who else writes the name', async ({ page }) => {
    await page.goto('#/rooms')
    await page.getByTestId('room-row').first().getByTestId('room-open').click()

    const detail = page.getByTestId('room-detail')
    // the three writers are named before anything is typed
    await expect(detail.getByTestId('room-writers-note')).toContainText('three writers')
    await expect(detail.getByTestId('room-writers-note')).toContainText('overwritten')

    await detail.getByTestId('room-name-input').fill('Cooking')
    await detail.getByTestId('room-name-save').click()

    await expect(page.getByTestId('toast-host')).toContainText('Rename drafted')

    // a rename is not free: the backend restarts and entity names change
    const impact = detail.getByTestId('room-impact')
    await expect(impact).toBeVisible()
    await expect(impact).toContainText('re-creates the zigbee2mqtt backend')
    await expect(impact).toContainText('Home Assistant entity names')
  })

  test('the room list and detail fit 390 px', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('#/rooms')
    await expect(page.getByTestId('rooms-list')).toBeVisible()

    let overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    )
    expect(overflow).toBe(false)

    await page.getByTestId('room-row').first().getByTestId('room-open').click()
    await expect(page.getByTestId('room-detail')).toBeVisible()

    overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    )
    expect(overflow).toBe(false)
  })
})
