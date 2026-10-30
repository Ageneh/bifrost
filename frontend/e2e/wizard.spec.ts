import { expect, test } from '@playwright/test'

/**
 * Wave 5's first run: from an empty configuration to a bridge that answers.
 *
 * The password step comes first and gates everything, so these tests start
 * after it. Seven steps, progress that survives a reload, and a Home that
 * says the bridge is configured once review has applied.
 */
test.beforeEach(async ({ request }) => {
  // no configuration at all: the wizard is due
  await request.post('__setup')
})

test.describe('first run, seven steps', () => {
  test('an unconfigured bridge opens the wizard, and it counts seven steps', async ({ page }) => {
    await page.goto('')

    await expect(page.getByTestId('wizard')).toBeVisible()
    await expect(page.getByTestId('wizard-step-count')).toHaveText('1 / 7')
    await expect(page.getByTestId('wizard-steps').locator('[data-testid^="wizard-step-"]')).toHaveCount(7)

    // the first step is the bridge's identity, not the network
    await expect(page.getByTestId('wizard-bridge-name')).toBeVisible()
    await expect(page.getByTestId('wizard-ip')).toHaveCount(0)
  })

  test('the wizard resumes at the same step after a reload', async ({ page }) => {
    await page.goto('')

    await page.getByTestId('wizard-bridge-mac').fill('00:11:22:33:44:55')
    await page.getByTestId('wizard-next').click()
    await expect(page.getByTestId('wizard-step-count')).toHaveText('2 / 7')
    await expect(page.getByTestId('wizard-ip')).toBeVisible()

    // a different session reads the record, not the page
    await page.reload()
    await expect(page.getByTestId('wizard-step-count')).toHaveText('2 / 7')
    await expect(page.getByTestId('wizard-ip')).toBeVisible()
  })

  test('every step up to review, then a configured Home', async ({ page }) => {
    await page.goto('')

    // 1 identity
    await page.getByTestId('wizard-bridge-mac').fill('00:11:22:33:44:55')
    await page.getByTestId('wizard-next').click()

    // 2 network
    await expect(page.getByTestId('wizard-step-count')).toHaveText('2 / 7')
    await page.getByTestId('wizard-ip').fill('10.0.0.2')
    await page.getByTestId('wizard-gateway').fill('10.0.0.1')
    await page.getByTestId('wizard-next').click()

    // 3 zigbee2mqtt
    await expect(page.getByTestId('wizard-step-count')).toHaveText('3 / 7')
    await page.getByTestId('wizard-server-url').fill('ws://10.0.0.100:8080/api')
    await page.getByTestId('wizard-next').click()

    // 4 discovery
    await expect(page.getByTestId('wizard-step-count')).toHaveText('4 / 7')
    await page.getByTestId('wizard-discover').click()
    await expect(page.getByTestId('wizard-discovery-list')).toBeVisible()
    await expect(page.getByTestId('wizard-discovered')).toHaveCount(2)
    await page.getByTestId('wizard-next').click()

    // 5 lights, every servable light chosen by default
    await expect(page.getByTestId('wizard-step-count')).toHaveText('5 / 7')
    await expect(page.getByTestId('wizard-light-0x0017880100000001')).toBeChecked()
    await page.getByTestId('wizard-light-0x0017880100000002').uncheck()
    await page.getByTestId('wizard-next').click()

    // 6 rooms
    await expect(page.getByTestId('wizard-step-count')).toHaveText('6 / 7')
    await expect(page.getByTestId('wizard-room-Kitchen')).toBeVisible()
    await page.getByTestId('wizard-next').click()

    // 7 review, which is the first and only write
    await expect(page.getByTestId('wizard-step-count')).toHaveText('7 / 7')
    await expect(page.getByTestId('wizard-summary')).toContainText('1 light served')
    await page.getByTestId('wizard-apply').click()

    // the bridge is configured, so Home answers and the wizard is gone
    await expect(page.getByTestId('home-view')).toBeVisible()
    await expect(page.getByTestId('wizard')).toHaveCount(0)
    await expect(page.getByTestId('home-attention-panel')).toBeVisible()
  })

  test('the wizard fits 390 px', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('')
    await expect(page.getByTestId('wizard')).toBeVisible()

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    )
    expect(overflow).toBe(false)
  })
})
