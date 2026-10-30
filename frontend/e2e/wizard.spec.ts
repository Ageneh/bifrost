import { expect, test } from '@playwright/test'

/**
 * The first run: from an empty configuration to a bridge that answers.
 *
 * The password step comes first and gates everything, so these tests start
 * after it. The step count is the wizard's own list, which wave 8 grew with
 * the entertainment question; progress survives a reload, and Home says the
 * bridge is configured once review has applied.
 */
test.beforeEach(async ({ request }) => {
  // no configuration at all: the wizard is due
  await request.post('__setup')
})

test.describe('first run, eight steps', () => {
  test('an unconfigured bridge opens the wizard, and it counts its own steps', async ({ page }) => {
    await page.goto('')

    await expect(page.getByTestId('wizard')).toBeVisible()
    await expect(page.getByTestId('wizard-step-count')).toHaveText('1 / 8')
    await expect(page.getByTestId('wizard-steps').locator('[data-testid^="wizard-step-"]')).toHaveCount(8)

    // the first step is the bridge's identity, not the network
    await expect(page.getByTestId('wizard-bridge-name')).toBeVisible()
    await expect(page.getByTestId('wizard-ip')).toHaveCount(0)
  })

  test('the wizard resumes at the same step after a reload', async ({ page }) => {
    await page.goto('')

    await page.getByTestId('wizard-bridge-mac').fill('00:11:22:33:44:55')
    await page.getByTestId('wizard-next').click()
    await expect(page.getByTestId('wizard-step-count')).toHaveText('2 / 8')
    await expect(page.getByTestId('wizard-ip')).toBeVisible()

    // a different session reads the record, not the page
    await page.reload()
    await expect(page.getByTestId('wizard-step-count')).toHaveText('2 / 8')
    await expect(page.getByTestId('wizard-ip')).toBeVisible()
  })

  test('every step up to review, then a configured Home', async ({ page }) => {
    await page.goto('')

    // 1 identity
    await page.getByTestId('wizard-bridge-mac').fill('00:11:22:33:44:55')
    await page.getByTestId('wizard-next').click()

    // 2 network
    await expect(page.getByTestId('wizard-step-count')).toHaveText('2 / 8')
    await page.getByTestId('wizard-ip').fill('10.0.0.2')
    await page.getByTestId('wizard-gateway').fill('10.0.0.1')
    await page.getByTestId('wizard-next').click()

    // 3 zigbee2mqtt
    await expect(page.getByTestId('wizard-step-count')).toHaveText('3 / 8')
    await page.getByTestId('wizard-server-url').fill('ws://10.0.0.100:8080/api')
    await page.getByTestId('wizard-next').click()

    // 4 discovery
    await expect(page.getByTestId('wizard-step-count')).toHaveText('4 / 8')
    await page.getByTestId('wizard-discover').click()
    await expect(page.getByTestId('wizard-discovery-list')).toBeVisible()
    await expect(page.getByTestId('wizard-discovered')).toHaveCount(6)
    await page.getByTestId('wizard-next').click()

    // 5 devices, every servable one chosen by default
    await expect(page.getByTestId('wizard-step-count')).toHaveText('5 / 8')
    await expect(page.getByTestId('wizard-device-0x0017880100000001')).toBeChecked()
    await page.getByTestId('wizard-device-0x0017880100000002').uncheck()
    await page.getByTestId('wizard-next').click()

    // 6 rooms
    await expect(page.getByTestId('wizard-step-count')).toHaveText('6 / 8')
    await expect(page.getByTestId('wizard-room-Kitchen')).toBeVisible()
    await page.getByTestId('wizard-next').click()

    // 7 entertainment: the question the recipe answers later
    await expect(page.getByTestId('wizard-step-count')).toHaveText('7 / 8')
    await expect(page.getByTestId('wizard-entertainment')).toBeVisible()
    await page.getByTestId('wizard-entertainment-ambilight').check()
    await page.getByTestId('wizard-next').click()

    // 8 review, which is the first and only write
    await expect(page.getByTestId('wizard-step-count')).toHaveText('8 / 8')
    await expect(page.getByTestId('wizard-summary')).toContainText('5 devices served')
    await expect(page.getByTestId('wizard-summary')).toContainText('A TV with Ambilight+Hue')
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
