#!/usr/bin/env node
/**
 * A static server for the built UI, standing in for the Rust one.
 *
 * It mirrors what `src/routes/ui.rs` does in production — injects a
 * `<base href>` and `window.__BIFROST_BASE__` from the path it is served
 * under, serves the shell for unknown non-API paths — and answers the few
 * admin endpoints the shell reads with fixed fixtures, so the end-to-end
 * test exercises a signed-in console behind an ingress prefix.
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

const DRAFT = {
  revision: 3,
  updated_at: '2026-10-30T10:00:00Z',
  config: { ...ACTIVE_CONFIG, rooms: { office: { name: 'Studio', icon: 'office' } } },
  validation: [],
}

const DIFF = [
  { path: 'rooms.office.name', before: 'Office', after: 'Studio', secret: false },
  { path: 'z2m.garage.token', before: null, after: null, secret: true },
]

const JSON_ROUTES = {
  'auth/session': {
    authenticated: true,
    setup_required: false,
    ingress: false,
    csrf: 'test-csrf',
  },
  'config': ACTIVE_CONFIG,
  'config/draft': DRAFT,
  'config/diff': DIFF,
  'config/validate': [],
  'service': SERVICES,
}

function send(response, status, body, type = 'application/json') {
  response.writeHead(status, {
    'content-type': type,
    'cache-control': 'no-store',
    'x-content-type-options': 'nosniff',
  })
  response.end(body)
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

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? '/', 'http://localhost')
  const path = url.pathname

  if (!path.startsWith(`${prefix}/`) && path !== prefix) {
    return send(response, 404, 'not found', 'text/plain')
  }

  const rest = path.slice(prefix.length).replace(/^\/+/, '')

  if (rest.startsWith('bifrost/')) {
    const endpoint = rest.slice('bifrost/'.length).replace(/\/+$/, '')
    if (endpoint in JSON_ROUTES) {
      return send(response, 200, JSON.stringify(JSON_ROUTES[endpoint]))
    }
    return send(response, 404, JSON.stringify({ error: 'not found', errors: [] }))
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
