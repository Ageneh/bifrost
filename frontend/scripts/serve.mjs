#!/usr/bin/env node
/**
 * A static server for the built UI, standing in for the Rust one.
 *
 * It mirrors what `src/routes/ui.rs` does in production — injects a
 * `<base href>` and `window.__BIFROST_BASE__` from the path it is served
 * under, serves the shell for unknown non-API paths — and answers the admin
 * endpoints the screens read and write with in-memory fixtures, so the
 * end-to-end tests exercise a signed-in console behind an ingress prefix
 * against state that behaves like the real draft: revision-checked writes,
 * a merge patch that follows RFC 7386, and an apply that consumes the draft.
 *
 * Usage: node scripts/serve.mjs [--port 4319] [--prefix /api/hassio_ingress/test]
 */
import { createReadStream } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import { createServer } from 'node:http'
import { extname, join, normalize } from 'node:path'

const PUBLIC_DIR = new URL('../.output/public/', import.meta.url).pathname

const args = process.argv.slice(2)
function argument(name, fallback) {
  const index = args.indexOf(name)
  return index >= 0 && args[index + 1] ? args[index + 1] : fallback
}

const port = Number(argument('--port', '4319'))
const prefix = argument('--prefix', '/api/hassio_ingress/test').replace(/\/+$/, '')
const base = `${prefix}/`

const CONTENT_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.woff2': 'font/woff2',
}

/** The active configuration, redacted, as the API returns it. */
const ACTIVE_CONFIG = {
  bridge: {
    name: 'Bifrost',
    mac: '00:11:22:33:44:55',
    ipaddress: '10.0.0.2',
    http_port: 80,
    https_port: 443,
    entm_port: 2100,
    netmask: '255.255.255.0',
    gateway: '10.0.0.1',
    timezone: 'Europe/Berlin',
  },
  z2m: {
    office: { url: 'ws://10.0.0.100:8080' },
    garage: { url: 'ws://10.0.0.101:8080' },
  },
  bifrost: { state_file: 'state.yaml', cert_file: 'cert.pem', auth: {} },
  rooms: { office: { name: 'Office', icon: 'office' } },
}

const SERVICES = {
  services: {
    '11111111-1111-4111-8111-111111111111': {
      id: '11111111-1111-4111-8111-111111111111',
      name: 'http',
      state: 'Running',
    },
    '22222222-2222-4222-8222-222222222222': {
      id: '22222222-2222-4222-8222-222222222222',
      name: 'z2m@office',
      state: 'Running',
    },
    '33333333-3333-4333-8333-333333333333': {
      id: '33333333-3333-4333-8333-333333333333',
      name: 'z2m@garage',
      state: 'Failed',
    },
  },
}

/** Mutable draft state, so a PATCH is visible on the next read. */
const draft = {
  revision: 3,
  updatedAt: '2026-10-30T10:00:00Z',
  config: null,
}

function initialDraftConfig() {
  const config = structuredClone(ACTIVE_CONFIG)
  config.rooms = { office: { name: 'Studio', icon: 'office' } }
  return config
}

draft.config = initialDraftConfig()

// ---- document helpers --------------------------------------------------

function isObject(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function leafPaths(document, prefixPath = '') {
  if (!isObject(document)) {
    return prefixPath === '' ? [] : [prefixPath]
  }
  const paths = []
  for (const key of Object.keys(document).sort()) {
    const path = prefixPath === '' ? key : `${prefixPath}.${key}`
    const value = document[key]
    if (isObject(value) && Object.keys(value).length > 0) {
      paths.push(...leafPaths(value, path))
    }
    else {
      paths.push(path)
    }
  }
  return paths
}

function getPath(document, path) {
  let current = document
  for (const key of path.split('.')) {
    if (!isObject(current) || !(key in current)) {
      return undefined
    }
    current = current[key]
  }
  return current
}

/** RFC 7386 merge patch, the same rule the Rust side applies. */
function mergePatch(target, patch) {
  if (!isObject(patch)) {
    return structuredClone(patch)
  }
  const result = isObject(target) ? target : {}
  for (const [key, value] of Object.entries(patch)) {
    if (value === null) {
      delete result[key]
    }
    else {
      result[key] = mergePatch(result[key], value)
    }
  }
  return result
}

function diffDocuments(before, after) {
  const paths = new Set([...leafPaths(before), ...leafPaths(after)])
  const changes = []
  for (const path of [...paths].sort()) {
    const from = getPath(before, path) ?? null
    const to = getPath(after, path) ?? null
    if (JSON.stringify(from) !== JSON.stringify(to)) {
      changes.push({ path, before: from, after: to, secret: false })
    }
  }
  return changes
}

// ---- responses ---------------------------------------------------------

function send(response, status, body, type = 'application/json') {
  response.writeHead(status, {
    'content-type': type,
    'cache-control': 'no-store',
    'x-content-type-options': 'nosniff',
  })
  response.end(body)
}

function sendJson(response, body) {
  send(response, 200, JSON.stringify(body))
}

function draftResponse() {
  return {
    revision: draft.revision,
    updated_at: draft.config === null ? null : draft.updatedAt,
    config: draft.config,
    validation: [],
  }
}

async function readJson(request) {
  const chunks = []
  for await (const chunk of request) {
    chunks.push(chunk)
  }
  const text = Buffer.concat(chunks).toString('utf8')
  return text === '' ? null : JSON.parse(text)
}

async function sendFile(response, path) {
  try {
    const info = await stat(path)
    if (!info.isFile()) {
      return false
    }
    response.writeHead(200, {
      'content-type': CONTENT_TYPES[extname(path)] ?? 'application/octet-stream',
      'cache-control': path.includes('/_nuxt/') ? 'public, max-age=31536000' : 'no-cache',
    })
    createReadStream(path).pipe(response)
    return true
  }
  catch {
    return false
  }
}

/** The shell, with the base and the base variable injected like the server does. */
async function sendShell(response) {
  const html = await readFile(join(PUBLIC_DIR, 'index.html'), 'utf8')
  const tags = `<base href="${base}"><script>window.__BIFROST_BASE__=${JSON.stringify(base)};</script>`
  const at = html.indexOf('<head>')
  const injected = at >= 0 ? `${html.slice(0, at + 6)}${tags}${html.slice(at + 6)}` : `${tags}${html}`
  send(response, 200, injected, 'text/html; charset=utf-8')
}

/** Every `/bifrost/*` endpoint the console uses, by method and path. */
async function handleApi(request, response, endpoint) {
  const method = request.method ?? 'GET'

  if (endpoint === 'auth/session') {
    return sendJson(response, {
      authenticated: true,
      setup_required: false,
      ingress: false,
      csrf: 'test-csrf',
    })
  }

  if (endpoint === 'config' && method === 'GET') {
    return sendJson(response, ACTIVE_CONFIG)
  }

  if (endpoint === 'service' && method === 'GET') {
    return sendJson(response, SERVICES)
  }

  if (endpoint.startsWith('service/') && method === 'PUT') {
    return sendJson(response, endpoint.slice('service/'.length))
  }

  if (endpoint === 'config/draft') {
    if (method === 'GET') {
      return sendJson(response, draftResponse())
    }
    if (method === 'PATCH' || method === 'PUT') {
      const body = await readJson(request)
      if (body !== null && body.revision !== draft.revision) {
        return send(response, 409, JSON.stringify({ error: 'The draft moved on', errors: [] }))
      }
      const base = draft.config ?? structuredClone(ACTIVE_CONFIG)
      draft.config
        = method === 'PATCH' ? mergePatch(structuredClone(base), body?.patch ?? {}) : body?.config
      draft.revision += 1
      return sendJson(response, draftResponse())
    }
    if (method === 'DELETE') {
      draft.config = null
      draft.revision += 1
      return sendJson(response, { revision: draft.revision })
    }
  }

  if (endpoint === 'config/diff' && method === 'GET') {
    return sendJson(response, diffDocuments(ACTIVE_CONFIG, draft.config ?? ACTIVE_CONFIG))
  }

  if (endpoint === 'config/validate' && method === 'POST') {
    return sendJson(response, [])
  }

  if (endpoint === 'config/apply' && method === 'POST') {
    const changes = diffDocuments(ACTIVE_CONFIG, draft.config ?? ACTIVE_CONFIG)
    const revision = draft.revision + 1
    draft.config = null
    draft.revision = revision
    return sendJson(response, {
      revision,
      plan: { hot: changes.map(change => ({ action: 'update', path: change.path })), restart_required: [] },
      changes,
    })
  }

  if (endpoint === 'config/restart' && method === 'POST') {
    return send(response, 204, '')
  }

  return send(response, 404, JSON.stringify({ error: 'not found', errors: [] }))
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? '/', 'http://localhost')
  const path = url.pathname

  if (!path.startsWith(`${prefix}/`) && path !== prefix) {
    return send(response, 404, 'not found', 'text/plain')
  }

  const rest = path.slice(prefix.length).replace(/^\/+/, '')

  if (rest.startsWith('bifrost/')) {
    const endpoint = rest.slice('bifrost/'.length).replace(/\/+$/, '')
    try {
      return await handleApi(request, response, endpoint)
    }
    catch (error) {
      return send(response, 500, JSON.stringify({ error: String(error), errors: [] }))
    }
  }

  if (rest !== '' && !rest.endsWith('/')) {
    const file = join(PUBLIC_DIR, normalize(rest).replace(/^(\.\.[/\\])+/, ''))
    if (await sendFile(response, file)) {
      return
    }
    // a path that names a file is a miss, not the shell
    if (rest.split('/').pop()?.includes('.')) {
      return send(response, 404, 'not found', 'text/plain')
    }
  }

  return sendShell(response)
})

server.listen(port, '127.0.0.1', () => {
  process.stdout.write(`serving ${PUBLIC_DIR} at http://127.0.0.1:${port}${base}\n`)
})
