/**
 * What the entertainment screens decide, apart from how they draw it.
 *
 * The preflight is the part that has to be right: each row says whether the
 * bridge can stream at all, and the row that is red names what to do about
 * it. The overlap check is the other half: a light belongs to one zone, so a
 * second zone that names it is refused here, before the Hue app finds out.
 */
import type {
  CertificateInfo,
  EntertainmentConfigurationSummary,
  EntertainmentDevice,
  EntertainmentZone,
  Service,
} from './generated'

/** How one preflight row reads. */
export type PreflightState = 'ok' | 'crit'

/** One line of the checklist in front of a recipe. */
export interface PreflightRow {
  id: string
  label: string
  state: PreflightState
  /** What the bridge sees, in the row's own terms. */
  detail: string
  /** The screen that fixes it, when one exists. */
  to?: string
}

/** Everything the preflight reads. */
export interface PreflightInput {
  servers: { name: string, online: boolean }[]
  devices: EntertainmentDevice[]
  maxStreams: number
  certificate: CertificateInfo | null
  services: Service[]
  /** The clock, so a test can age a certificate. */
  now?: Date
}

/** A light two zones would share. */
export interface ZoneOverlap {
  /** The light service, which is what both zones name. */
  light: string
  /** The light's name, for the sentence that refuses it. */
  name: string
  /** The zone that holds it now. */
  zone: string
  zoneId: string
}

/**
 * The five things a stream needs, in the order they break.
 *
 * A row is green only when the bridge can prove it; "the TV can reach it" is
 * not something the bridge can see, so the HTTPS row reports the listener and
 * the certificate, and says so.
 */
export function derivePreflight(input: PreflightInput): PreflightRow[] {
  const now = input.now ?? new Date()

  const online = input.servers.filter(server => server.online)
  const z2m: PreflightRow = online.length > 0
    ? {
        id: 'z2m',
        label: 'zigbee2mqtt is connected',
        state: 'ok',
        detail: online.map(server => server.name).join(', '),
        to: '/system',
      }
    : {
        id: 'z2m',
        label: 'zigbee2mqtt is connected',
        state: 'crit',
        detail: 'No server is online, so no light can receive a frame.',
        to: '/system',
      }

  const capable = input.devices
  const light: PreflightRow = capable.length > 0
    ? {
        id: 'entertainment-light',
        label: 'A light can render a stream',
        state: 'ok',
        detail: capable.map(device => device.name).join(', '),
        to: '/devices',
      }
    : {
        id: 'entertainment-light',
        label: 'A light can render a stream',
        state: 'crit',
        detail: 'No device on this bridge reports an entertainment service.',
        to: '/devices',
      }

  const streams: PreflightRow = input.maxStreams >= 1
    ? {
        id: 'max-streams',
        label: 'The bridge allows a stream',
        state: 'ok',
        detail: `${input.maxStreams} stream${input.maxStreams === 1 ? '' : 's'} allowed (bifrost.entertainment.max_streams)`,
        to: '/system/bridge',
      }
    : {
        id: 'max-streams',
        label: 'The bridge allows a stream',
        state: 'crit',
        detail: 'bifrost.entertainment.max_streams is 0, so every stream is refused.',
        to: '/system/bridge',
      }

  const certificate = input.certificate
  const starts = certificate === null ? null : new Date(certificate.not_before)
  const ends = certificate === null ? null : new Date(certificate.not_after)
  const certificateRow: PreflightRow = certificate !== null
    && starts !== null && ends !== null
    && !Number.isNaN(starts.valueOf()) && !Number.isNaN(ends.valueOf())
    && starts <= now && now <= ends
    ? {
        id: 'certificate',
        label: 'The certificate is valid',
        state: 'ok',
        detail: `${certificate.common_name}, until ${ends.toISOString().slice(0, 10)}`,
        to: '/system/bridge',
      }
    : {
        id: 'certificate',
        label: 'The certificate is valid',
        state: 'crit',
        detail: certificate === null
          ? 'The bridge did not answer with its certificate.'
          : `Valid from ${certificate.not_before} to ${certificate.not_after}.`,
        to: '/system/bridge',
      }

  // the listener is what the bridge can see; whether the TV reaches it is
  // what the hardware acceptance is for
  const https = input.services.find(service => service.name.startsWith('https'))
  const httpsRow: PreflightRow = https?.state === 'Running'
    ? {
        id: 'https',
        label: 'The HTTPS listener is running',
        state: 'ok',
        detail: 'The Sync box reaches this port; the bridge cannot see the TV side.',
        to: '/system',
      }
    : {
        id: 'https',
        label: 'The HTTPS listener is running',
        state: 'crit',
        detail: https === undefined
          ? 'No https service is registered.'
          : `The https service is ${String(https.state)}.`,
        to: '/system',
      }

  return [z2m, light, streams, certificateRow, httpsRow]
}

/**
 * The lights a new zone would take from a zone that already holds them.
 *
 * A light may sit in one zone only: the stream-to-zone rule has no way to
 * tell two zones that share a light apart, so the editor refuses the overlap
 * and names both sides instead of saving a zone that would half work.
 */
export function findOverlaps(
  selected: string[],
  devices: EntertainmentDevice[],
  zones: EntertainmentConfigurationSummary[],
  names: Map<string, string>,
): ZoneOverlap[] {
  const overlaps: ZoneOverlap[] = []

  for (const id of selected) {
    const device = devices.find(entry => entry.id === id)
    const light = device?.light
    if (light === null || light === undefined) {
      continue
    }

    for (const zone of zones) {
      if (!zone.lights.includes(light)) {
        continue
      }
      overlaps.push({
        light,
        name: names.get(id) ?? device?.name ?? id,
        zone: zone.name,
        zoneId: zone.id,
      })
    }
  }

  return overlaps
}

/** The members of a zone, one entry per service, in location order. */
export interface ZoneMember {
  service: string
  /** The device's name, when the capabilities still list it. */
  name: string
  segments: number
  positions: { x: number, y: number, z: number }[]
  /** False when the device is missing from the inventory or reports offline. */
  online: boolean
}

/**
 * The zone's services joined with what the bridge knows about them.
 *
 * A zone outlives the device list: a device can be removed, and a light can
 * be offline. Both still hold a position, so both are listed — offline is a
 * state of the row, not a reason to hide it.
 */
export function zoneMembers(
  zone: EntertainmentZone,
  devices: EntertainmentDevice[],
  availability: Map<string, boolean>,
): ZoneMember[] {
  return zone.locations.map((location) => {
    const device = devices.find(entry => entry.id === location.service)
    const online = device === undefined
      ? false
      : availability.get(device.id) ?? true
    return {
      service: location.service,
      name: device?.name ?? location.service,
      segments: Math.max(device?.segments ?? location.positions.length, 1),
      positions: location.positions,
      online,
    }
  })
}
