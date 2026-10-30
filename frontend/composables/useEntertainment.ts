/**
 * The entertainment data every screen in the mode reads.
 *
 * Capabilities, the certificate and the service list are fetched once and
 * shared; a zone is read on its own because it is what the editor changes.
 * Nothing here polls: the streams have their own composable, and the socket
 * says when the set of streams changed.
 */
import { computed, readonly, ref } from 'vue'
import type {
  CertificateInfo,
  DeviceInfo,
  EntertainmentCapabilities,
  EntertainmentDevice,
  EntertainmentZone,
  Service,
} from '~/lib/generated'
import { ApiError, api } from '~/lib/api'
import { useInventory } from '~/composables/useInventory'
import { derivePreflight, findOverlaps, zoneMembers } from '~/lib/entertainment'
import type { PreflightRow, ZoneMember, ZoneOverlap } from '~/lib/entertainment'

const capabilities = ref<EntertainmentCapabilities | null>(null)
const certificate = ref<CertificateInfo | null>(null)
const services = ref<Service[]>([])
const loading = ref(false)
const failure = ref<string | null>(null)

/** A position the editor writes back. */
export interface Position {
  x: number
  y: number
  z: number
}

/** A service as the editor writes it into a zone. */
export interface ZoneService {
  service: string
  positions: Position[]
}

export function useEntertainment() {
  const inventory = useInventory()

  /** Read what the mode needs; calling it twice is two reads, not a cache. */
  async function load(): Promise<void> {
    loading.value = true
    try {
      const [caps, cert, list] = await Promise.all([
        api.entertainmentCapabilities(),
        api.certificate().catch(() => null),
        api.services(),
      ])
      capabilities.value = caps
      certificate.value = cert
      services.value = Object.values(list.services)
      failure.value = null
      await inventory.refresh()
    }
    catch (error) {
      failure.value = error instanceof ApiError ? error.message : 'Entertainment could not be read.'
    }
    finally {
      loading.value = false
    }
  }

  /** The devices that can render a stream, or an empty list before the read. */
  const devices = computed<EntertainmentDevice[]>(() => capabilities.value?.devices ?? [])

  /** The zones, as the picker lists them. */
  const zones = computed(() => capabilities.value?.configurations ?? [])

  /**
   * Whether each entertainment device is reachable.
   *
   * The inventory keys devices by IEEE address and the zone names light
   * services, so the join is through the light a capability reports. A device
   * the inventory no longer holds counts as offline, which is what the canvas
   * draws.
   */
  const availability = computed<Map<string, boolean>>(() => {
    const map = new Map<string, boolean>()
    for (const device of devices.value) {
      const light = device.light
      if (light === null) {
        continue
      }
      const known = inventory.devices.value.find(entry =>
        entry.resources.some(resource => resource.rid === light),
      )
      map.set(device.id, known?.availability === 'online')
    }
    return map
  })

  /** The inventory row behind an entertainment device, for Identify. */
  function inventoryDevice(service: string): DeviceInfo | undefined {
    const device = devices.value.find(entry => entry.id === service)
    if (device?.light === null || device?.light === undefined) {
      return undefined
    }
    return inventory.devices.value.find(entry =>
      entry.resources.some(resource => resource.rid === device.light),
    )
  }

  /** The preflight rows for a recipe, in the order they break. */
  const preflight = computed<PreflightRow[]>(() => derivePreflight({
    servers: inventory.servers.value,
    devices: devices.value,
    maxStreams: capabilities.value?.max_streams ?? 0,
    certificate: certificate.value,
    services: services.value,
  }))

  /** True when every preflight row is green. */
  const ready = computed(() => preflight.value.every(row => row.state === 'ok'))

  /** One zone, with its members joined to what the bridge knows. */
  async function zone(id: string): Promise<EntertainmentZone> {
    return await api.entertainmentZone(id)
  }

  /** The members of a zone, with their positions and whether they are online. */
  function members(zone: EntertainmentZone): ZoneMember[] {
    return zoneMembers(zone, devices.value, availability.value)
  }

  /** The lights a new zone would take from one that already holds them. */
  function overlaps(selected: string[]): ZoneOverlap[] {
    const names = new Map(devices.value.map(device => [device.id, device.name]))
    return findOverlaps(selected, devices.value, zones.value, names)
  }

  /**
   * Save a zone's services and positions.
   *
   * This is the only write the editor makes: members and positions, through
   * the v2 `PUT`. The channel list is the server's answer to that, never a
   * second thing written here.
   */
  async function save(id: string, services: ZoneService[]): Promise<void> {
    await api.putZone(id, {
      locations: {
        service_locations: services.map(entry => ({
          positions: entry.positions,
          service: { rid: entry.service, rtype: 'entertainment' },
        })),
      },
    })
  }

  /** Create a zone over the chosen devices and return its id. */
  async function create(name: string, chosen: string[]): Promise<string> {
    return await api.createZone({
      configuration_type: 'screen',
      metadata: { name },
      stream_proxy: null,
      locations: {
        service_locations: chosen.map(service => ({
          // no positions: the server lays the segments out and the editor
          // reads that layout back, so the default exists in one place
          positions: [] as Position[],
          service: { rid: service, rtype: 'entertainment' },
        })),
      },
    })
  }

  /** Flash one device, so a person can tell which bulb a position means. */
  async function identify(service: string): Promise<void> {
    const device = inventoryDevice(service)
    if (device === undefined) {
      throw new ApiError(404, 'This device is not in the inventory, so it cannot flash.')
    }
    await inventory.identify(device)
  }

  return {
    capabilities: readonly(capabilities),
    certificate: readonly(certificate),
    services: readonly(services),
    loading: readonly(loading),
    failure: readonly(failure),
    devices,
    zones,
    availability,
    preflight,
    ready,
    load,
    zone,
    members,
    overlaps,
    save,
    create,
    identify,
    inventoryDevice,
  }
}
