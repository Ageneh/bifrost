import { mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { expect, test } from '@playwright/test'

/**
 * The screenshots `doc/ui.md` shows.
 *
 * They come from this run, against the same fixture server and the same
 * 390 px project the rest of the suite uses, so a screenshot cannot show a
 * screen the tests never reach: the assertion above each capture fails the
 * run first. The files are written to `doc/images/`, which is where the
 * document links them from.
 */

const here = dirname(fileURLToPath(import.meta.url))
const images = join(here, '../../doc/images')

// The screens are asserted on every run; the files are rewritten only when
// asked, because a suite that rewrites tracked files on every run leaves the
// tree dirty for nothing. Refresh them with BIFROST_DOC_IMAGES=1.
const rewrite = process.env.BIFROST_DOC_IMAGES === '1'

async function capture(page: import('@playwright/test').Page, name: string) {
  if (!rewrite) return
  await mkdir(images, { recursive: true })
  // the viewport, not the whole page: the bottom nav is fixed, and a
  // full-page capture would draw it again in the middle of the scroll
  await page.screenshot({ path: join(images, `${name}.jpg`), type: 'jpeg', quality: 82 })
}

// one pixel per CSS pixel: the images are read in a document, not on a
// retina screen, and 3x would triple the file size for nothing
test.use({ deviceScaleFactor: 1 })

test.beforeEach(async ({ request }) => {
  await request.post('__reset')
})

test.describe('the screens the documentation shows', () => {
  test('home, devices, rooms, entertainment, pairing, system', async ({ page }) => {
    await page.goto('')
    await expect(page.getByTestId('home-view')).toBeVisible()
    await capture(page, 'home')

    await page.goto('#/devices')
    await expect(page.getByTestId('device-table')).toBeVisible()
    await capture(page, 'devices')

    await page.goto('#/rooms')
    await expect(page.getByTestId('rooms-list')).toBeVisible()
    await capture(page, 'rooms')

    await page.goto('#/entertainment')
    await expect(page.getByTestId('entertainment-home')).toBeVisible()
    await capture(page, 'entertainment')

    await page.goto('#/pairing')
    await expect(page.getByTestId('pairing-screen')).toBeVisible()
    await capture(page, 'pairing')

    await page.goto('#/system/bridge')
    await expect(page.getByTestId('bridge-screen')).toBeVisible()
    await capture(page, 'system-bridge')
  })

  test('the first-run wizard', async ({ page, request }) => {
    await request.post('__setup')
    await page.goto('#/setup')
    await expect(page.getByTestId('wizard')).toBeVisible()
    await capture(page, 'wizard')
  })
})
