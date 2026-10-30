import { expect, test } from '@playwright/test'

/**
 * Wave 8's entertainment editor, behind the ingress prefix at 390 px.
 *
 * The recipes, the wizard's entertainment step (in wizard.spec.ts), the
 * canvas with its numeric inputs, the derived channel table, the diagnostics
 * panel, and the overlap refusal that keeps a light in one zone.
 */
const ZONE_TV = '2f1c4f1e-0000-4000-8000-000000000001'
const ZONE_DESK = '2f1c4f1e-0000-4000-8000-000000000002'
const TV_LEFT = '2f1c4f1e-0000-4000-8000-0000000000a1'
const TV_RIGHT = '2f1c4f1e-0000-4000-8000-0000000000a2'
const DESK_LIGHT = '2f1c4f1e-0000-4000-8000-0000000000a3'
const STUDIO_LIGHT = '2f1c4f1e-0000-4000-8000-0000000000a4'

test.beforeEach(async ({ request }) => {
  await request.post('__reset')
})

test.describe('entertainment', () => {
  test('the mode lists the preflight, the recipes and the zones', async ({ page }) => {
    await page.goto('#/entertainment')

    await expect(page.getByTestId('entertainment-home')).toBeVisible()
    await expect(page.getByTestId('nav-entertainment')).toHaveAttribute('data-state', 'active')

    // every preflight row is green against the fixture bridge
    for (const row of ['z2m', 'entertainment-light', 'max-streams', 'certificate', 'https']) {
      await expect(page.getByTestId(`preflight-${row}`)).toHaveAttribute('data-state', 'ok')
    }

    await expect(page.getByTestId('recipe-ambilight')).toHaveAttribute('data-state', 'ready')
    await expect(page.getByTestId('recipe-sync-box')).toBeVisible()
    await expect(page.getByTestId('recipe-second-zone')).toBeVisible()

    await expect(page.getByTestId(`zone-${ZONE_TV}`)).toBeVisible()
    await expect(page.getByTestId(`zone-${ZONE_DESK}`)).toBeVisible()
  })

  test('a recipe creates a zone and opens its editor', async ({ page }) => {
    await page.goto('#/entertainment')

    await page.getByTestId('recipe-second-zone-start').click()
    await expect(page.getByTestId('entertainment-wizard')).toBeVisible()

    // the TV lights and the desk light are in a zone already; the studio
    // light is the one this zone may take
    await page.getByTestId('entertainment-wizard-name').fill('Study')
    await page.getByTestId(`entertainment-wizard-pick-${STUDIO_LIGHT}`).check()
    await page.getByTestId('entertainment-wizard-create').click()

    await expect(page.getByTestId('zone-editor')).toBeVisible()
    await expect(page.getByTestId('zone-name')).toHaveText('Study')
    await expect(page.getByTestId('channel-table')).toBeVisible()
  })

  test('a light already in a zone is refused, and both names are given', async ({ page }) => {
    await page.goto('#/entertainment')

    await page.getByTestId('recipe-ambilight-start').click()
    await page.getByTestId(`entertainment-wizard-pick-${TV_LEFT}`).check()

    const overlap = page.getByTestId('entertainment-wizard-overlap')
    await expect(overlap).toBeVisible()
    await expect(overlap).toContainText('TV left')
    await expect(overlap).toContainText('Living room TV')
    await expect(page.getByTestId('entertainment-wizard-create')).toBeDisabled()
  })

  test('the canvas edits numbers and the channel table follows the save', async ({ page }) => {
    // positions are edited at a desk width; 390 px is the read-only case
    await page.setViewportSize({ width: 1200, height: 900 })
    await page.goto(`#/entertainment/${ZONE_TV}`)

    await expect(page.getByTestId('zone-canvas')).toHaveAttribute('data-state', 'editable')
    await expect(page.getByTestId('channel-table')).toBeVisible()
    await expect(page.getByTestId('zone-save')).toBeDisabled()

    // the saved position is the one the zone holds
    await expect(page.getByTestId(`position-${TV_LEFT}-0-x`)).toHaveValue('-0.4')

    await page.getByTestId(`position-${TV_LEFT}-0-x`).fill('-0.2')
    await page.getByTestId(`position-${TV_LEFT}-0-x`).blur()
    await expect(page.getByTestId('zone-save')).toBeEnabled()

    await page.getByTestId('zone-save').click()
    await expect(page.getByTestId('toast-ok').first()).toContainText('Zone saved.')

    // the channel the server derived from the saved position
    await expect(page.getByTestId('channel-row-0')).toContainText('-0.2')
  })

  test('an offline light keeps its position and is drawn hollow', async ({ page }) => {
    await page.goto(`#/entertainment/${ZONE_DESK}`)

    await expect(page.getByTestId('zone-editor')).toBeVisible()
    await expect(page.getByTestId(`position-dot-${DESK_LIGHT}-0`)).toHaveAttribute(
      'data-state',
      'offline',
    )
    await expect(page.getByTestId(`position-${DESK_LIGHT}-0-x`)).toBeVisible()
  })

  test('the diagnostics panel says why a stream ended', async ({ page }) => {
    await page.goto('#/entertainment')

    await expect(page.getByTestId('diagnostics-table')).toBeVisible()
    await expect(page.getByTestId(`diagnostics-row-${ZONE_TV}`)).toContainText('50')
    await expect(page.getByTestId(`diagnostics-error-${ZONE_DESK}`)).toContainText(
      'Timeout while waiting for sync data',
    )

    // the status column stops one stream and leaves the other
    await page.getByTestId(`entertainment-streams-stop-${ZONE_TV}`).click()
    await expect(page.getByTestId(`diagnostics-row-${ZONE_TV}`)).toHaveCount(0)
    await expect(page.getByTestId(`diagnostics-row-${ZONE_DESK}`)).toBeVisible()
  })

  test('the canvas is read-only at 390 px', async ({ page }) => {
    await page.goto(`#/entertainment/${ZONE_TV}`)

    await expect(page.getByTestId('zone-canvas')).toHaveAttribute('data-state', 'read-only')
    await expect(page.getByTestId(`position-${TV_LEFT}-0-x`)).toBeDisabled()
    await expect(page.getByTestId('zone-save')).toBeDisabled()

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    )
    expect(overflow).toBe(false)
  })

  test('the editor identifies a device by its light', async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 900 })
    await page.goto(`#/entertainment/${ZONE_TV}`)

    const identify = page.getByTestId(`zone-identify-${TV_RIGHT}`)
    await expect(identify).toBeVisible()
    await identify.click()
    await expect(page.getByTestId('toast-ok').first()).toContainText('flashing')
  })
})
