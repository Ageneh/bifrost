/**
 * The device inventory, kept live from the socket.
 *
 * One store for the whole app: the list is fetched once and then maintained
 * from the `Inventory` frames the socket pushes, so a removal or an
 * availability flip is on screen in the same second it happened. A socket
 * that reconnects gets a fresh snapshot from the server, which is also what
 * a lagging subscriber is sent, so the store never has to reconcile.
 */
import { computed, readonly, ref, shallowRef } from 'vue'
import type {
  DeviceEvent,
  DeviceInfo,
  DeviceType,
  InventorySnapshot,
  PermitJoinResponse,
  ServerInfo,
} from '~/lib/generated'
import { ApiError, api } from '~/lib/api'
import { useLiveSocket } from '~/composables/useLiveSocket'

// shallow: the arrays are replaced, never mutated in place, and a deep ref
// would have to unwrap the recursive JSON a reading carries
const devices = shallowRef<DeviceInfo[]>([])
const servers = shallowRef<ServerInfo[]>([])
const updated = ref<string | null>(null)
const loading = ref(false)
const failure = ref<string | null>(null)

/**
 * The last few things devices reported, newest first.
 *
 * The inventory holds the current state; this holds the change, so a screen
 * can show "21.4 °C at 14:02" and a remote's presses without polling. Capped
 * because nothing on screen reads further back than the strip it fills.
 */
const events = shallowRef<DeviceEvent[]>([])
const EVENT_LIMIT = 200

let listening = false

function replaceAll(snapshot: InventorySnapshot): void {
  devices.value = snapshot.devices
  servers.value = snapshot.servers
  updated.value = snapshot.updated
}

/** Apply one pushed change to the store. */
function apply(update: {
  kind: 'snapshot' | 'device' | 'removed' | 'server'
  [key: string]: unknown
}): void {
  switch (update.kind) {
    case 'snapshot':
      replaceAll(update as unknown as InventorySnapshot)
      break
    case 'device': {
      const device = update as unknown as DeviceInfo
      const index = devices.value.findIndex(
        entry => entry.server === device.server && entry.ieee === device.ieee,
      )
      if (index >= 0) {
        const next = [...devices.value]
        next[index] = device
        devices.value = next
      }
      else {
        devices.value = [...devices.value, device]
      }
      break
    }
    case 'removed': {
      const { server, ieee } = update as unknown as { server: string, ieee: string }
      devices.value = devices.value.filter(
        entry => !(entry.server === server && entry.ieee === ieee),
      )
      break
    }
    case 'server': {
      const server = update as unknown as ServerInfo
      const index = servers.value.findIndex(entry => entry.name === server.name)
      if (index >= 0) {
        const next = [...servers.value]
        next[index] = server
        servers.value = next
      }
      else {
        servers.value = [...servers.value, server]
      }
      break
    }
  }
}

/** Follow the socket once, however many components ask for the store. */
function listen(): void {
  if (listening) {
    return
  }
  listening = true
  useLiveSocket().onUpdate((update) => {
    if ('Inventory' in update) {
      apply(update.Inventory as never)
    }
    if ('DeviceEvent' in update) {
      events.value = [update.DeviceEvent as DeviceEvent, ...events.value].slice(0, EVENT_LIMIT)
    }
  })
}

export function useInventory() {
  /** Read the list again; the socket keeps it current from then on. */
  async function refresh(): Promise<void> {
    loading.value = true
    try {
      replaceAll(await api.devices())
      failure.value = null
      listen()
    }
    catch (error) {
      failure.value = error instanceof ApiError ? error.message : 'The devices could not be read'
    }
    finally {
      loading.value = false
    }
  }

  /** The number of devices in each tab, for the tab badges. */
  const counts = computed<Record<string, number>>(() => {
    const counts: Record<string, number> = {}
    for (const tab of DEVICE_TABS) {
      counts[tab.key] = devices.value.filter(device => tab.types.includes(device.device_type)).length
    }
    return counts
  })

  /** Everything a device reported, newest first. */
  function eventsFor(ieee: string): DeviceEvent[] {
    return events.value.filter(event => event.ieee === ieee)
  }

  /** The most recent reading a device of `property`, if it has one. */
  function readingOf(device: DeviceInfo, property: string) {
    return device.readings.find(reading => reading.property === property)
  }

  /** The last action a remote sent, from its reading or the live events. */
  function lastAction(device: DeviceInfo): string | null {
    const reading = readingOf(device, 'action')
    if (reading !== undefined) {
      return String(reading.value)
    }
    const event = events.value.find(entry => entry.ieee === device.ieee && entry.kind === 'action')
    return event === undefined ? null : String(event.value)
  }

  /** Every device whose battery is low enough to need a new one. */
  const lowBattery = computed(() =>
    devices.value.filter(device => device.battery !== null && device.battery < LOW_BATTERY),
  )

  /** The server a device came from, when the inventory knows it. */
  function serverInfo(name: string): ServerInfo | undefined {
    return servers.value.find(entry => entry.name === name)
  }

  /** Flash a device and report what the bridge answered. */
  async function identify(device: DeviceInfo): Promise<void> {
    await api.identifyDevice(device.ieee, device.server)
  }

  /** Open one server's join window, and read the state back. */
  async function permitJoin(name: string): Promise<PermitJoinResponse> {
    const window = await api.permitJoin(name)
    await refresh()
    return window
  }

  return {
    // the arrays are replaced, never mutated in place, so handing them out
    // as they are costs nothing
    devices,
    servers,
    counts,
    events: readonly(events),
    lowBattery,
    updated: readonly(updated),
    loading: readonly(loading),
    failure: readonly(failure),
    serverInfo,
    eventsFor,
    readingOf,
    lastAction,
    refresh,
    identify,
    permitJoin,
  }
}

/** The battery level below which a device needs attention, in percent. */
export const LOW_BATTERY = 15

/**
 * One tab of the Devices table.
 *
 * Climate is one tab for the two things a room's climate is: a temperature
 * and a light level. They are different device classes and different Hue
 * resources, and a person reads them as one column.
 */
export interface DeviceTab {
  key: string
  label: string
  types: DeviceType[]
}

/** The tabs, in the order the screen shows them. */
export const DEVICE_TABS: DeviceTab[] = [
  { key: 'light', label: 'Lights', types: ['light'] },
  { key: 'plug', label: 'Plugs', types: ['plug'] },
  { key: 'motion', label: 'Motion', types: ['motion'] },
  { key: 'climate', label: 'Climate', types: ['temperature', 'illuminance'] },
  { key: 'contact', label: 'Contact', types: ['contact'] },
  { key: 'remote', label: 'Remotes', types: ['remote'] },
  { key: 'other', label: 'Other', types: ['other'] },
]

/** The tab a device class belongs to. */
export function tabOf(type: DeviceType): DeviceTab | undefined {
  return DEVICE_TABS.find(tab => tab.types.includes(type))
}
