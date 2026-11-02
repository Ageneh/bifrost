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
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import { createServer, request as httpRequest } from 'node:http'
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

/**
 * When set, the shell and every static asset come from this origin — the
 * release binary — instead of `PUBLIC_DIR`. The admin API stays on the
 * fixtures: the real bridge needs a zigbee network and hardware behind it,
 * while the artefact under test here is the bundle the binary embeds.
 */
const upstream = argument('--upstream', process.env.BIFROST_E2E_UPSTREAM ?? '').replace(/\/+$/, '')

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
let ACTIVE_CONFIG = {
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

/** The device inventory, as the API returns it. */
const SERVERS = [
  {
    name: 'office',
    online: true,
    permit_join: { time: 0, device: null, opened_at: null },
    last_seen: '2026-10-30T09:00:00Z',
  },
  {
    name: 'garage',
    online: false,
    permit_join: { time: 0, device: null, opened_at: null },
    last_seen: '2026-10-30T08:00:00Z',
  },
]

function device(server, ieee, name, type, supported, overrides = {}) {
  return {
    server,
    ieee,
    friendly_name: name,
    model: 'LCT015',
    vendor: 'Signify',
    description: null,
    device_type: type,
    supported,
    imported: supported,
    unsupported_reason: supported ? null : 'Bifrost has no Hue mapping for plugs',
    battery: null,
    readings: [],
    exposes: [{ property: 'state', kind: 'binary', access: 7, unit: null, values: ['ON', 'OFF'] }],
    capabilities: {
      dimmable: supported,
      color: supported,
      color_temperature: supported,
      gradient: false,
      effects: false,
      motion: type === 'motion',
      temperature: false,
      illuminance: false,
      contact: type === 'contact',
      action: false,
    },
    availability: 'online',
    resources: [{ rid: `device-${ieee}`, rtype: 'device' }],
    room: null,
    group: null,
    last_seen: '2026-10-30T09:00:00Z',
    ...overrides,
  }
}

const DEVICES = [
  device('office', '0x0017880100000001', 'Kitchen ceiling', 'light', true, {
    room: { rid: 'room-kitchen', name: 'Kitchen' },
    group: { id: 1, name: 'Kitchen' },
    resources: [
      { rid: 'device-0x0017880100000001', rtype: 'device' },
      { rid: 'light-a1', rtype: 'light' },
    ],
  }),
  // in a group but in no room: the chain breaks at the room hop
  device('office', '0x0017880100000002', 'Hall lamp', 'light', true, {
    group: { id: 2, name: 'Hall' },
    resources: [
      { rid: 'device-0x0017880100000002', rtype: 'device' },
      { rid: 'light-a2', rtype: 'light' },
    ],
  }),
  device('office', '0x00158d0000000010', 'Desk plug', 'plug', false),
  device('office', '0x00158d0000000050', 'Back door', 'contact', true, {
    battery: 87,
    capabilities: { contact: true },
    readings: [
      { property: 'contact', value: false, at: new Date(Date.now() - 180_000).toISOString() },
    ],
  }),
  // its server is not running: the chain breaks at the z2m hop
  device('garage', '0x00158d0000000020', 'Hall motion', 'motion', true, {
    availability: 'unknown',
    battery: 64,
    capabilities: { motion: true },
    readings: [
      { property: 'occupancy', value: true, at: new Date(Date.now() - 5_000).toISOString() },
    ],
  }),
  device('office', '0x00158d0000000030', 'Bedroom climate', 'temperature', true, {
    battery: 12,
    capabilities: { temperature: true },
    readings: [
      { property: 'temperature', value: 21.4, at: new Date(Date.now() - 60_000).toISOString() },
    ],
  }),
  device('office', '0x00158d0000000060', 'Bedside switch', 'remote', true, {
    battery: 91,
    capabilities: { action: true },
    readings: [
      { property: 'action', value: 'single', at: new Date(Date.now() - 900_000).toISOString() },
    ],
  }),
]

/** Every open console socket, so a test can push an event through one. */
const consoleSockets = new Set()

/** One text frame, as RFC 6455 frames it. */
function textFrame(payload) {
  const body = Buffer.from(JSON.stringify(payload))
  const header = body.length < 126
    ? Buffer.from([0x81, body.length])
    : Buffer.from([0x81, 126, body.length >> 8, body.length & 0xFF])
  return Buffer.concat([header, body])
}

/** Push one update to every open console. */
function broadcast(update) {
  const frame = textFrame(update)
  for (const socket of consoleSockets) {
    socket.write(frame)
  }
}

/** The devices the query leaves, as the API filters them. */
function filteredDevices(search) {
  const server = search.get('server')
  const type = search.get('type')
  const supported = search.get('supported')

  return DEVICES
    .filter(entry => !server || entry.server === server)
    .filter(entry => !type || entry.device_type === type)
    .filter(entry => supported === null || String(entry.supported) === supported)
}

/** The rooms the bridge holds, as the admin API reports them. */
const ROOMS = [
  {
    id: 'room-kitchen',
    topic: 'Kitchen',
    name: 'Kitchen',
    icon: 'kitchen',
    devices: [
      { ieee: '0x0017880100000001', name: 'Kitchen ceiling', supported: true },
    ],
    scenes: [
      { id: 'scene-bright', name: 'Bright', active: true },
      { id: 'scene-relax', name: 'Relax', active: false },
    ],
  },
  {
    id: 'room-hall',
    topic: 'Hall',
    name: 'Hall',
    icon: 'hallway',
    devices: [
      { ieee: '0x0017880100000002', name: 'Hall lamp', supported: true },
    ],
    scenes: [],
  },
]

const SERVICES = {
  services: {
    '11111111-1111-4111-8111-111111111111': {
      id: '11111111-1111-4111-8111-111111111111',
      name: 'http',
      state: 'Running',
    },
    '44444444-4444-4444-8444-444444444444': {
      id: '44444444-4444-4444-8444-444444444444',
      name: 'https',
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

/** True while the fixture stands in for a bridge with no configuration. */
/**
 * The entertainment zones and the streams running on them.
 *
 * Two streams, which is the point of the wave: the TV and a second client at
 * once. A stop removes one row and leaves the other, and `__reset` puts both
 * back.
 */
const ZONE_DEVICES = [
  { id: '2f1c4f1e-0000-4000-8000-0000000000a1', name: 'TV left', segments: 1, light: 'light-a1' },
  { id: '2f1c4f1e-0000-4000-8000-0000000000a2', name: 'TV right', segments: 1, light: 'light-a2' },
  // not in the inventory: the canvas draws it hollow, which is the offline case
  { id: '2f1c4f1e-0000-4000-8000-0000000000a3', name: 'Desk light', segments: 1, light: 'light-a3' },
  // in no zone: the free light a recipe can take
  { id: '2f1c4f1e-0000-4000-8000-0000000000a4', name: 'Studio light', segments: 1, light: 'light-a4' },
]

/**
 * The default layout, the same rule the server applies: the segments spread
 * evenly along the top edge, left to right.
 */
function autoLayout(segments) {
  const round = value => Math.round(value * 1000) / 1000
  if (segments <= 0) {
    return []
  }
  if (segments === 1) {
    return [{ x: 0, y: 0.8, z: 0.4 }]
  }
  const step = 0.8 / (segments - 1)
  return Array.from({ length: segments }, (_, index) => ({
    x: round(-0.4 + step * index),
    y: 0.8,
    z: 0.4,
  }))
}

/** The channels a zone's locations derive, in channel order. */
function channelsOf(zone) {
  const channels = []
  let id = 0
  for (const location of zone.locations) {
    const device = ZONE_DEVICES.find(entry => entry.id === location.service)
    const count = Math.max(device?.segments ?? 1, 1)
    const positions = location.positions.length > 0 ? location.positions : autoLayout(count)
    for (let index = 0; index < count; index += 1) {
      channels.push({
        channel_id: id,
        position: positions[index] ?? positions[0] ?? { x: 0, y: 0, z: 0 },
        members: [{ service: location.service, index }],
      })
      id += 1
    }
  }
  return channels
}

const INITIAL_ZONES = [
  {
    id: '2f1c4f1e-0000-4000-8000-000000000001',
    name: 'Living room TV',
    type: 'screen',
    status: 'active',
    locations: [
      { service: ZONE_DEVICES[0].id, positions: [{ x: -0.4, y: 0.8, z: 0.4 }] },
      { service: ZONE_DEVICES[1].id, positions: [{ x: 0.4, y: 0.8, z: 0.4 }] },
    ],
  },
  {
    id: '2f1c4f1e-0000-4000-8000-000000000002',
    name: 'Desk monitor',
    type: 'monitor',
    status: 'active',
    locations: [
      { service: '2f1c4f1e-0000-4000-8000-0000000000a3', positions: [{ x: 0, y: 0.8, z: 0.4 }] },
    ],
  },
]

let ZONES = structuredClone(INITIAL_ZONES)

function initialStreams() {
  return [
    {
      area: ZONES[0].id,
      started: '2026-10-30T09:00:00Z',
      fps_in: 50,
      fps_out: 20,
      frames: 12_000,
      drops: 7_200,
      last_error: null,
    },
    {
      area: ZONES[1].id,
      started: '2026-10-30T09:05:00Z',
      fps_in: 30,
      fps_out: 20,
      frames: 3_600,
      drops: 1_200,
      // a session that ended badly stays listed with its reason, which is
      // what the diagnostics panel exists to show
      last_error: 'Timeout while waiting for sync data',
    },
  ]
}

let streams = initialStreams()

let setupMode = false

/** The first-run wizard's own record, beside the draft. */
let wizard = { step: 'identity', values: {}, updated_at: null }

/** Mutable draft state, so a PATCH is visible on the next read. */
const draft = {
  revision: 3,
  updatedAt: '2026-10-30T10:00:00Z',
  config: null,
}

/**
 * The pairing window, as the real server keeps it: a deadline, and a read
 * that compares it with the clock.
 */
const linkButton = { deadline: 0, window: 30 }

/** The pairing log the screen shows, newest first. */
function initialPairingRecords() {
  return [
    { devicetype: 'Hue Sync Box', at: '2026-10-30T09:30:00Z', accepted: true },
    { devicetype: 'hue app#2.0.0#iPhone14,3', at: '2026-10-30T09:10:00Z', accepted: false },
  ]
}
let pairingRecords = initialPairingRecords()

/** What the certificate panel reads. */
const CERTIFICATE = {
  common_name: '001122fffe334455',
  bridge_id: '001122fffe334455',
  mac: '00:11:22:33:44:55',
  not_before: '2017-01-01T00:00:00Z',
  not_after: '2038-01-19T03:14:07Z',
  serial: '001122fffe334455',
  fingerprint: 'AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99',
}

/** What the state summary reads. */
const STATE_SUMMARY = {
  bridge_id: '001122fffe334455',
  mac: '00:11:22:33:44:55',
  name: 'Bifrost',
  version: '1.0.0',
  counts: {
    lights: 3,
    grouped_lights: 2,
    rooms: 1,
    devices: 5,
    scenes: 2,
    entertainment_configurations: 0,
  },
}

function linkButtonStatus() {
  const remaining = linkButton.deadline === 0 ? 0 : Math.ceil((linkButton.deadline - Date.now()) / 1000)
  if (remaining <= 0) {
    linkButton.deadline = 0
    return { pressed: false, remaining_seconds: 0 }
  }
  return { pressed: true, remaining_seconds: remaining }
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
async function handleApi(request, response, endpoint, search) {
  const method = request.method ?? 'GET'

  if (endpoint === 'devices' && method === 'GET') {
    return sendJson(response, {
      servers: SERVERS,
      devices: filteredDevices(search),
      updated: '2026-10-30T09:00:00Z',
    })
  }

  if (endpoint.startsWith('devices/') && method === 'GET') {
    const ieee = decodeURIComponent(endpoint.slice('devices/'.length))
    const found = DEVICES.find(entry => entry.ieee === ieee)
    return found
      ? sendJson(response, found)
      : send(response, 404, JSON.stringify({ error: 'No such device', errors: [] }))
  }

  if (endpoint.startsWith('devices/') && endpoint.endsWith('/identify') && method === 'POST') {
    const ieee = decodeURIComponent(endpoint.slice('devices/'.length, -'/identify'.length))
    const found = DEVICES.find(entry => entry.ieee === ieee)
    return found
      ? sendJson(response, found)
      : send(response, 404, JSON.stringify({ error: 'No such device', errors: [] }))
  }

  if (endpoint.startsWith('z2m/') && endpoint.endsWith('/permit_join')) {
    const name = decodeURIComponent(endpoint.slice('z2m/'.length, -'/permit_join'.length))
    if (method === 'POST') {
      const body = (await readJson(request)) ?? {}
      return sendJson(response, { server: name, time: body.time ?? 60, device: body.device ?? null })
    }
    return sendJson(response, { server: name, time: 0, device: null })
  }

  if (endpoint === 'auth/session') {
    return sendJson(response, {
      authenticated: true,
      setup_required: false,
      ingress: false,
      csrf: 'test-csrf',
    })
  }

  if (endpoint === 'config' && method === 'GET') {
    if (setupMode) {
      return send(response, 404, JSON.stringify({ error: 'No configuration yet', errors: [] }))
    }
    return sendJson(response, ACTIVE_CONFIG)
  }

  if (endpoint === 'setup/wizard') {
    if (method === 'PUT') {
      const body = await readJson(request)
      wizard = { step: body.step, values: body.values ?? {}, updated_at: '2026-10-30T11:00:00Z' }
      return sendJson(response, wizard)
    }
    return sendJson(response, wizard)
  }

  if (endpoint === 'setup/discovery' && method === 'POST') {
    return sendJson(response, {
      devices: DEVICES.filter(entry => entry.supported).map(entry => ({
        ieee: entry.ieee,
        friendly_name: entry.friendly_name,
        model: entry.model,
        vendor: entry.vendor,
        supported: entry.supported,
      })),
      rooms: [{ topic: 'Kitchen', members: ['0x0017880100000001'] }],
    })
  }

  if (endpoint === 'rooms' && method === 'GET') {
    return sendJson(response, { rooms: ROOMS })
  }

  const recall = endpoint.match(/^rooms\/([^/]+)\/scenes\/([^/]+)\/recall$/)
  if (recall && method === 'POST') {
    const room = ROOMS.find(entry => entry.id === decodeURIComponent(recall[1]))
    if (!room) {
      return send(response, 404, JSON.stringify({ error: 'No such room', errors: [] }))
    }
    for (const scene of room.scenes) {
      scene.active = scene.id === decodeURIComponent(recall[2])
    }
    return send(response, 200, JSON.stringify(null))
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
    // a configuration written by setup becomes the running one, and the
    // wizard is over
    if (setupMode && draft.config !== null) {
      ACTIVE_CONFIG = draft.config
      setupMode = false
    }
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

  // ---- pairing ---------------------------------------------------------

  if (endpoint === 'bridge/link-button') {
    if (method === 'POST') {
      const body = (await readJson(request)) ?? {}
      const seconds = body.seconds ?? 30
      if (seconds < 1 || seconds > 120) {
        return send(
          response,
          422,
          JSON.stringify({ error: 'Open the pairing window for 1 to 120 seconds', errors: [] }),
        )
      }
      linkButton.window = seconds
      linkButton.deadline = Date.now() + seconds * 1000
      return sendJson(response, linkButtonStatus())
    }
    return sendJson(response, linkButtonStatus())
  }

  if (endpoint === 'bridge/pairing' && method === 'GET') {
    return sendJson(response, {
      mode: ACTIVE_CONFIG.pairing?.require_link_button ? 'required' : 'compat',
      records: pairingRecords,
    })
  }

  // ---- identity --------------------------------------------------------

  if (endpoint === 'bridge/certificate' && method === 'GET') {
    return sendJson(response, CERTIFICATE)
  }

  if (endpoint === 'bridge/certificate/regenerate' && method === 'POST') {
    return sendJson(response, { ...CERTIFICATE, fingerprint: `${CERTIFICATE.fingerprint}FF` })
  }

  if (endpoint === 'bridge/identity/backup' && method === 'GET') {
    return sendJson(response, {
      version: 1,
      bridge_id: CERTIFICATE.bridge_id,
      certificate_pem: '-----BEGIN PRIVATE KEY-----\nfixture\n-----END PRIVATE KEY-----\n',
      paired_clients: pairingRecords,
    })
  }

  if (endpoint === 'bridge/identity/mac' && method === 'POST') {
    const body = (await readJson(request)) ?? {}
    if (body.confirm !== CERTIFICATE.bridge_id) {
      return send(
        response,
        422,
        JSON.stringify({ error: `Type the bridge id (${CERTIFICATE.bridge_id}) to change the MAC address`, errors: [] }),
      )
    }
    return sendJson(response, { mac: body.mac, bridge_id: '001122fffe334466', restart_required: true })
  }

  if (endpoint === 'bridge/identity/restore' && method === 'POST') {
    const body = (await readJson(request)) ?? {}
    if (!body.admin_password) {
      return send(response, 403, JSON.stringify({ error: 'The admin password does not match', errors: [] }))
    }
    pairingRecords = body.bundle?.paired_clients ?? pairingRecords
    return sendJson(response, {
      bridge_id: body.bundle?.bridge_id ?? CERTIFICATE.bridge_id,
      paired_clients: pairingRecords.length,
      tokens_written: [],
    })
  }

  // ---- entertainment ---------------------------------------------------

  if (endpoint === 'entertainment/capabilities' && method === 'GET') {
    return sendJson(response, {
      max_streams: 2,
      devices: ZONE_DEVICES.map(device => ({ ...device, configurable: false })),
      configurations: ZONES.map(zone => ({
        id: zone.id,
        name: zone.name,
        configuration_type: zone.type,
        status: zone.status,
        channels: channelsOf(zone).length,
        lights: zone.locations
          .map(location => ZONE_DEVICES.find(device => device.id === location.service)?.light)
          .filter(light => light !== undefined),
      })),
    })
  }

  if (endpoint === 'entertainment/status' && method === 'GET') {
    return sendJson(response, { max_streams: 2, streams })
  }

  if (endpoint.startsWith('entertainment/configurations/') && method === 'GET') {
    const id = decodeURIComponent(endpoint.slice('entertainment/configurations/'.length))
    const zone = ZONES.find(entry => entry.id === id)
    if (zone === undefined) {
      return send(response, 404, JSON.stringify({ error: 'No such zone', errors: [] }))
    }
    return sendJson(response, {
      id: zone.id,
      name: zone.name,
      configuration_type: zone.type,
      status: zone.status,
      active_streamer: streams.some(stream => stream.area === zone.id) ? 'fixture-app' : null,
      locations: zone.locations,
      channels: channelsOf(zone),
    })
  }

  if (endpoint.startsWith('entertainment/') && endpoint.endsWith('/stop') && method === 'POST') {
    const area = decodeURIComponent(endpoint.slice('entertainment/'.length, -'/stop'.length))
    const before = streams.length
    streams = streams.filter(stream => stream.area !== area)
    return before === streams.length
      ? send(response, 404, JSON.stringify({ error: 'No such stream', errors: [] }))
      : sendJson(response, null)
  }

  if (endpoint === 'state/summary' && method === 'GET') {
    return sendJson(response, STATE_SUMMARY)
  }

  if (endpoint === 'state/export' && method === 'GET') {
    return send(response, 200, 'version: 1\nres: {}\n', 'application/yaml')
  }

  if (endpoint === 'state/reset' && method === 'POST') {
    const body = (await readJson(request)) ?? {}
    if (body.confirm !== STATE_SUMMARY.bridge_id) {
      return send(
        response,
        422,
        JSON.stringify({ error: `Type the bridge id (${STATE_SUMMARY.bridge_id}) to reset the state`, errors: [] }),
      )
    }
    return sendJson(response, { ...STATE_SUMMARY, counts: { lights: 0, grouped_lights: 0, rooms: 0, devices: 2, scenes: 0, entertainment_configurations: 0 } })
  }

  return send(response, 404, JSON.stringify({ error: 'not found', errors: [] }))
}

/**
 * The CLIP v2 surface the editor writes through.
 *
 * Only entertainment configurations: creating one and moving its lights.
 * The answer shapes are the Rust ones — a `{data, errors}` envelope — so the
 * client cannot tell this apart from the bridge.
 */
async function handleClip(request, response, path) {
  const method = request.method ?? 'GET'

  if (path === 'resource/entertainment_configuration' && method === 'POST') {
    const body = (await readJson(request)) ?? {}
    const id = `2f1c4f1e-0000-4000-8000-${String(Date.now()).padStart(12, '0').slice(-12)}`
    ZONES.push({
      id,
      name: body.metadata?.name ?? 'Zone',
      type: body.configuration_type ?? 'screen',
      status: 'inactive',
      locations: (body.locations?.service_locations ?? []).map(location => ({
        service: location.service.rid,
        positions: location.positions ?? [],
      })),
    })
    return sendJson(response, { data: [{ rid: id, rtype: 'entertainment_configuration' }], errors: [] })
  }

  const put = path.match(/^resource\/entertainment_configuration\/([^/]+)$/)
  if (put && method === 'PUT') {
    const id = decodeURIComponent(put[1])
    const zone = ZONES.find(entry => entry.id === id)
    if (zone === undefined) {
      return send(response, 404, JSON.stringify({ data: [], errors: [{ description: 'No such zone' }] }))
    }
    const body = (await readJson(request)) ?? {}
    if (body.locations?.service_locations !== undefined) {
      zone.locations = body.locations.service_locations.map(location => ({
        service: location.service.rid,
        positions: location.positions ?? [],
      }))
    }
    return sendJson(response, { data: [{ rid: id, rtype: 'entertainment_configuration' }], errors: [] })
  }

  return send(response, 404, JSON.stringify({ data: [], errors: [{ description: 'not found' }] }))
}

/**
 * Forward `rest` to the release binary unchanged, headers included: the
 * binary injects `<base>` and `window.__BIFROST_BASE__` from `X-Ingress-Path`,
 * which is exactly what the prefix tests read back.
 */
function proxyToUpstream(request, response, rest) {
  const target = new URL(`${upstream}/${rest}`)
  const headers = { ...request.headers }
  delete headers.host
  // Home Assistant's ingress proxy sets this on the way in; without it the
  // binary cannot know the prefix and injects a root `<base>`, which sends
  // every relative asset to the host root. Overwrite whatever the client
  // sent: the prefix is this proxy's decision, not the browser's.
  headers['x-ingress-path'] = prefix

  const forwarded = httpRequest(target, { method: request.method, headers }, (answer) => {
    response.writeHead(answer.statusCode ?? 502, answer.headers)
    answer.pipe(response)
  })
  forwarded.on('error', () => {
    send(response, 502, 'the release binary did not answer', 'text/plain')
  })
  request.pipe(forwarded)
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? '/', 'http://localhost')
  const path = url.pathname

  // the shell is served at `/` too, as the real server does: a reload after
  // a hash navigation lands there, because a hash carries no path
  if (!path.startsWith(`${prefix}/`) && path !== prefix) {
    if (path === '/' || path === '') {
      return sendShell(response)
    }
    return send(response, 404, 'not found', 'text/plain')
  }

  const rest = path.slice(prefix.length).replace(/^\/+/, '')

  // the fixture state is one process, so each test starts from the same
  // draft rather than from whatever the previous test left behind
  // push one device event to every open console, which is how a live
  // reading arrives without a real bridge
  if (rest === '__event' && request.method === 'POST') {
    const chunks = []
    for await (const chunk of request) {
      chunks.push(chunk)
    }
    const event = JSON.parse(Buffer.concat(chunks).toString() || '{}')
    broadcast({
      DeviceEvent: {
        ieee: event.ieee ?? '0x00158d0000000020',
        kind: event.kind ?? 'occupancy',
        value: event.value ?? true,
        at: event.at ?? new Date().toISOString(),
      },
    })
    return send(response, 204, '')
  }

  if (rest === '__reset' && request.method === 'POST') {
    setupMode = false
    wizard = { step: 'identity', values: {}, updated_at: null }
    linkButton.deadline = 0
    linkButton.window = 30
    pairingRecords = initialPairingRecords()
    streams = initialStreams()
    ZONES = structuredClone(INITIAL_ZONES)
    draft.config = initialDraftConfig()
    draft.revision = 3
    for (const room of ROOMS) {
      room.scenes.forEach((scene, index) => {
        scene.active = index === 0
      })
    }
    return send(response, 204, '')
  }

  // the same fixture, with no configuration at all: the first-run wizard is
  // due and every screen but setup answers 404
  if (rest === '__setup' && request.method === 'POST') {
    setupMode = true
    wizard = { step: 'identity', values: {}, updated_at: null }
    draft.config = null
    draft.revision = 0
    return send(response, 204, '')
  }

  // the shell and the assets are the release binary's; the admin endpoints
  // below stay fixtures because they need a zigbee network behind them
  if (upstream && !rest.startsWith('bifrost/') && !rest.startsWith('clip/v2/')) {
    return proxyToUpstream(request, response, rest)
  }

  if (rest.startsWith('clip/v2/')) {
    try {
      return await handleClip(request, response, rest.slice('clip/v2/'.length).replace(/\/+$/, ''))
    }
    catch (error) {
      return send(response, 500, JSON.stringify({ data: [], errors: [{ description: String(error) }] }))
    }
  }

  if (rest.startsWith('bifrost/')) {
    const endpoint = rest.slice('bifrost/'.length).replace(/\/+$/, '')
    try {
      return await handleApi(request, response, endpoint, url.searchParams)
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

// the console's one socket: `/bifrost/ws`, upgraded by hand so the fixture
// needs no dependency
server.on('upgrade', (request, socket) => {
  const key = request.headers['sec-websocket-key']
  if (typeof key !== 'string') {
    socket.destroy()
    return
  }
  const accept = createHash('sha1')
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest('base64')
  socket.write([
    'HTTP/1.1 101 Switching Protocols',
    'Upgrade: websocket',
    'Connection: Upgrade',
    `Sec-WebSocket-Accept: ${accept}`,
    '',
    '',
  ].join('\r\n'))
  consoleSockets.add(socket)
  socket.on('close', () => consoleSockets.delete(socket))
  socket.on('error', () => consoleSockets.delete(socket))
  // nothing the console sends changes the fixture
  socket.on('data', () => {})
})

server.listen(port, '127.0.0.1', () => {
  process.stdout.write(`serving ${PUBLIC_DIR} at http://127.0.0.1:${port}${base}\n`)
})
