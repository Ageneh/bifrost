#!/usr/bin/env node
/**
 * Make the generated HTML reference its assets relatively.
 *
 * Nuxt emits `/_nuxt/...` even with `app.baseURL: './'`, and an absolute
 * path breaks the moment the app is served under an ingress prefix: the
 * browser asks the proxy for `/_nuxt/...` instead of `<prefix>/_nuxt/...`.
 * The server injects a `<base href>` from the ingress path, so `./_nuxt/...`
 * resolves correctly both at `/` and behind the prefix.
 *
 * Run by `pnpm build` after `nuxt generate`. It fails when there is nothing
 * to rewrite, so a future Nuxt that changes its output cannot pass silently.
 */
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

const PUBLIC_DIR = new URL('../.output/public/', import.meta.url).pathname
const FILES = ['index.html', '200.html', '404.html']

/** Absolute references that must become document-relative. */
const REPLACEMENTS = [
  ['href="/_nuxt/', 'href="./_nuxt/'],
  ['src="/_nuxt/', 'src="./_nuxt/'],
  ['href="/favicon.', 'href="./favicon.'],
  // Nuxt normalises `app.baseURL: './'` back to '/' and bakes it into the
  // runtime config; the client resolves lazy chunks against it
  ['baseURL:"/"', 'baseURL:"./"'],
  ['buildAssetsDir:"/_nuxt/"', 'buildAssetsDir:"_nuxt/"'],
]

let rewritten = 0
for (const name of FILES) {
  const path = join(PUBLIC_DIR, name)
  let html
  try {
    html = await readFile(path, 'utf8')
  }
  catch (error) {
    if (error.code === 'ENOENT') {
      continue
    }
    throw error
  }

  let next = html
  for (const [from, to] of REPLACEMENTS) {
    next = next.split(from).join(to)
  }

  if (next === html) {
    if (name === 'index.html') {
      throw new Error(`${name} had no absolute asset references to rewrite; check the Nuxt output`)
    }
    continue
  }

  await writeFile(path, next)
  rewritten += 1
}

if (rewritten === 0) {
  throw new Error('no HTML file was rewritten; check the Nuxt output')
}

process.stdout.write(`rewrote ${rewritten} HTML file(s) to relative asset paths\n`)
