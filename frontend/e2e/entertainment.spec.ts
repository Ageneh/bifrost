import { expect, test } from '@playwright/test'

/**
 * Wave 8's entertainment recipes, behind the ingress prefix at 390 px.
 *
 * The wizard's entertainment step is in wizard.spec.ts; the canvas, the
 * channel table and the diagnostics panel arrive with their own commits.
 */
const ZONE_TV = '2f1c4f1e-0000-4000-8000-000000000001'
const ZONE_DESK = '2f1c4f1e-0000-4000-8000-000000000002'
const TV_LEFT = '2f1c4f1e-0000-4000-8000-0000000000a1'
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

    // the recipe hands over to the zone it made
    await expect(page).toHaveURL(/#\/entertainment\/[0-9a-f-]+$/)

    await page.goto('#/entertainment')
    await expect(page.getByTestId('entertainment-zones')).toContainText('Study')
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
})
