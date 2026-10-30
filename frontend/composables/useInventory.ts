/**
 * The device inventory, kept live from the socket.
 *
 * One store for the whole app: the list is fetched once and then maintained
 * from the `Inventory` frames the socket pushes, so a removal or an
 * availability flip is on screen in the same second it happened. A socket
 * that reconnects gets a fresh snapshot from the server, which is also what
 * a lagging subscriber is sent, so the store never has to reconcile.
 */
import { computed, readonly, ref } from 'vue'
import type {
  DeviceInfo,
  DeviceType,
  InventorySnapshot,
  PermitJoinResponse,
  ServerInfo,
} from '~/lib/generated'
import { ApiError, api } from '~/lib/api'
import { useLiveSocket } from '~/composables/useLiveSocket'

const devices = ref<DeviceInfo[]>([])
const servers = ref<ServerInfo[]>([])
const updated = ref<string | null>(null)
const loading = ref(false)
const failure = ref<string | null>(null)

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
        devices.value = devices.value.with(index, device)
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
        servers.value = servers.value.with(index, server)
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

  /** The number of devices in each class, for the type tabs. */
  const counts = computed<Record<string, number>>(() => {
    const counts: Record<string, number> = {}
    for (const device of devices.value) {
      counts[device.device_type] = (counts[device.device_type] ?? 0) + 1
    }
    return counts
  })

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
    updated: readonly(updated),
    loading: readonly(loading),
    failure: readonly(failure),
    serverInfo,
    refresh,
    identify,
    permitJoin,
  }
}

/** The classes, in the order the tabs show them. */
export const DEVICE_TYPES: Array<{ key: DeviceType, label: string }> = [
  { key: 'light', label: 'Lights' },
  { key: 'plug', label: 'Plugs' },
  { key: 'motion', label: 'Motion' },
  { key: 'temperature', label: 'Temperature' },
  { key: 'illuminance', label: 'Illuminance' },
  { key: 'contact', label: 'Contact' },
  { key: 'remote', label: 'Remotes' },
  { key: 'other', label: 'Other' },
]
