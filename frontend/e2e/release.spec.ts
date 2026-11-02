import { expect, test } from '@playwright/test'
import { createHash } from 'node:crypto'
import { readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * The release binary, not a dev server.
 *
 * `scripts/e2e-release.sh` builds the frontend, builds the Rust binary with
 * that bundle embedded, boots it, and points this fixture server's shell and
 * asset routes at it. Every other spec then reads the binary's HTML and its
 * fingerprinted chunks while the admin API stays on the fixtures — the real
 * bridge needs a zigbee network and hardware behind it.
 *
 * The failure this file exists for: a green `cargo check` while the embedded
 * asset directory is empty, or a `.output` left over from an earlier build.
 * Each served chunk is hashed and compared with the bundle just built.
 */

const UPSTREAM = process.env.BIFROST_E2E_UPSTREAM ?? ''
const PUBLIC_DIR = fileURLToPath(new URL('../.output/public/', import.meta.url))
const PREFIX = '/api/hassio_ingress/test'

test.skip(UPSTREAM === '', 'run scripts/e2e-release.sh: it boots the release binary')

async function bundleFiles(): Promise<string[]> {
  const files: string[] = []
  async function walk(directory: string, prefix: string): Promise<void> {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const relative = prefix === '' ? entry.name : `${prefix}/${entry.name}`
      if (entry.isDirectory()) {
        await walk(join(directory, entry.name), relative)
      }
      else {
        files.push(relative)
      }
    }
  }
  await walk(join(PUBLIC_DIR, '_nuxt'), '')
  return files.sort()
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex')
}

test.describe('the release binary serves the bundle it was built with', () => {
  test('every embedded chunk hashes to the chunk just built', async ({ request }) => {
    const files = await bundleFiles()
    expect(files.length).toBeGreaterThan(0)

    for (const file of files) {
      const built = await readFile(join(PUBLIC_DIR, '_nuxt', file))
      const served = await request.get(`./_nuxt/${file}`)
      expect(served.status(), `_nuxt/${file}`).toBe(200)
      expect(sha256(await served.body()), `_nuxt/${file}`).toBe(sha256(built))
    }
  })

  test('the shell is the binary\'s, behind the ingress prefix', async ({ page }) => {
    await page.goto('')

    const base = await page.evaluate(
      () => (globalThis as { __BIFROST_BASE__?: string }).__BIFROST_BASE__,
    )
    expect(base).toBe(`${PREFIX}/`)

    const shell = await page.content()
    expect(shell).toContain(`<base href="${PREFIX}/">`)

    // and the chunks it loaded came from the prefix, not the host root
    const assetUrls = await page.evaluate(() =>
      performance
        .getEntriesByType('resource')
        .map(entry => entry.name)
        .filter(name => name.includes('_nuxt')),
    )
    expect(assetUrls.length).toBeGreaterThan(0)
    for (const url of assetUrls) {
      expect(url).toContain(`${PREFIX}/_nuxt/`)
    }
  })

  test('every route renders at 390 px', async ({ page }) => {
    const routes = ['', '#/devices', '#/rooms', '#/entertainment', '#/pairing', '#/system']

    for (const route of routes) {
      await page.goto(route)
      await expect(page.getByTestId('app-shell'), route).toBeVisible()

      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth > window.innerWidth,
      )
      expect(overflow, route).toBe(false)
    }
  })
})
