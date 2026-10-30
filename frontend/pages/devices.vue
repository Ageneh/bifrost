<script setup lang="ts">
/**
 * Every device on the zigbee network, what it can do, and why one of them
 * is missing from the Hue app.
 *
 * One table with a tab per device class, three filters over it, and the
 * selected device in the status column. A row Bifrost cannot serve reads
 * `not mapped` and opens the same detail, where the chain names the hop that
 * breaks. The only action on a device is identify; pairing lives in the
 * head, because it belongs to the server and not to a row.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import DataTable from '~/components/DataTable.vue'
import type { TableColumn } from '~/components/DataTable.vue'
import CapabilityBadges from '~/components/CapabilityBadges.vue'
import DeviceDetail from '~/components/DeviceDetail.vue'
import DeviceFilterBar from '~/components/DeviceFilterBar.vue'
import { useToast } from '~/composables/useToast'
import { DEVICE_TABS, LOW_BATTERY, useInventory } from '~/composables/useInventory'
import { useStatusPanel } from '~/composables/useStatusPanel'
import type { DeviceInfo } from '~/lib/generated'

const {
  devices,
  servers,
  loading,
  failure,
  counts,
  serverInfo,
  refresh,
  readingOf,
  lastAction,
  identify: identifyDevice,
  permitJoin: openJoinWindow,
} = useInventory()
const statusPanel = useStatusPanel()
const toast = useToast()

const type = ref<string>('all')
const search = ref('')
const server = ref('')
const supported = ref<'all' | 'yes' | 'no'>('all')

const selected = ref<DeviceInfo | null>(null)

const joining = ref(false)
const joinServer = ref('')

const columns: TableColumn[] = [
  { key: 'friendly_name', label: 'Device' },
  { key: 'readout', label: 'Reading' },
  { key: 'device_type', label: 'Type' },
  { key: 'model', label: 'Model' },
  { key: 'availability', label: 'Reach' },
  { key: 'mapping', label: 'Hue' },
]

/** The readout a row was built with. */
function rowReadout(row: Record<string, unknown>): { text: string, age: string, warn: string } {
  return row.readout as { text: string, age: string, warn: string }
}

/** How long ago a reading arrived, in words a person reads at a glance. */
function ago(at: string | null | undefined): string {
  if (at === null || at === undefined) {
    return ''
  }
  const seconds = Math.max(0, Math.round((Date.now() - new Date(at).getTime()) / 1000))
  if (seconds < 60) {
    return `${seconds} s`
  }
  if (seconds < 3600) {
    return `${Math.round(seconds / 60)} min`
  }
  if (seconds < 86400) {
    return `${Math.round(seconds / 3600)} h`
  }
  return `${Math.round(seconds / 86400)} d`
}

/**
 * The one thing a sensor row says about itself.
 *
 * A sensor has one reading a person cares about at a glance — is there
 * motion, how warm is it, is the door open — and the age of it, because a
 * reading from an hour ago is not the same as one from a second ago.
 */
function readout(device: DeviceInfo): { text: string, age: string, warn: string } {
  const batteryWarn = device.battery !== null && device.battery < LOW_BATTERY
    ? `battery ${device.battery} %`
    : ''

  switch (device.device_type) {
    case 'motion': {
      const reading = readingOf(device, 'occupancy')
      if (reading === undefined) {
        return { text: 'no reading yet', age: '', warn: batteryWarn }
      }
      return {
        text: reading.value === true ? 'motion' : 'clear',
        age: ago(reading.at),
        warn: batteryWarn,
      }
    }
    case 'temperature': {
      const reading = readingOf(device, 'temperature')
      if (reading === undefined) {
        return { text: 'no reading yet', age: '', warn: batteryWarn }
      }
      return { text: `${Number(reading.value).toFixed(1)} °C`, age: ago(reading.at), warn: batteryWarn }
    }
    case 'illuminance': {
      const reading = readingOf(device, 'illuminance')
      if (reading === undefined) {
        return { text: 'no reading yet', age: '', warn: batteryWarn }
      }
      return { text: `${Math.round(Number(reading.value))} lx`, age: ago(reading.at), warn: batteryWarn }
    }
    case 'contact': {
      const reading = readingOf(device, 'contact')
      if (reading === undefined) {
        return { text: 'no reading yet', age: '', warn: batteryWarn }
      }
      return {
        text: reading.value === true ? 'open' : 'closed',
        age: ago(reading.at),
        warn: batteryWarn,
      }
    }
    case 'remote': {
      const action = lastAction(device)
      return { text: action ?? 'no press yet', age: '', warn: batteryWarn }
    }
    default:
      return { text: '', age: '', warn: batteryWarn }
  }
}

/** The devices the tabs, the filters and the search leave. */
const rows = computed(() => {
  const needle = search.value.trim().toLowerCase()

  return devices.value
    .filter((device) => {
      if (type.value === 'all') {
        return true
      }
      const tab = DEVICE_TABS.find(entry => entry.key === type.value)
      return tab !== undefined && tab.types.includes(device.device_type)
    })
    .filter(device => server.value === '' || device.server === server.value)
    .filter(device => supported.value === 'all'
      || (supported.value === 'yes') === device.supported)
    .filter((device) => {
      if (needle === '') {
        return true
      }
      return [device.friendly_name, device.model, device.vendor, device.ieee]
        .some(value => (value ?? '').toLowerCase().includes(needle))
    })
    .map(device => ({ ...device, readout: readout(device) } as Record<string, unknown>))
})

const total = computed(() => devices.value.length)
const shown = computed(() => rows.value.length)

/** The server whose join window the head opens. */
const joinable = computed(() => servers.value.filter(entry => entry.online))

/** A device the person picked, refreshed when the inventory changes it. */
function select(device: DeviceInfo): void {
  selected.value = device
  showDetail()
}

function showDetail(): void {
  const device = selected.value
  if (device === null) {
    statusPanel.clear()
    return
  }
  statusPanel.show(DeviceDetail, {
    device,
    server: serverInfo(device.server),
    onIdentify: identify,
    onClose: () => {
      selected.value = null
      statusPanel.clear()
    },
  })
}

async function identify(device: DeviceInfo): Promise<void> {
  try {
    await identifyDevice(device)
  }
  catch (error) {
    toast.fail(`The device did not flash: ${error instanceof Error ? error.message : String(error)}`)
  }
}

/** Open the join window of one server, from the head. */
async function permitJoin(): Promise<void> {
  const target = joinServer.value || joinable.value[0]?.name
  if (target === undefined) {
    return
  }

  joining.value = true
  try {
    const window = await openJoinWindow(target)
    toast.ok(`Join window open on ${target} for ${window.time} seconds`)
  }
  catch (error) {
    toast.fail(`The join window did not open: ${error instanceof Error ? error.message : String(error)}`)
  }
  finally {
    joining.value = false
  }
}

/** Keep the status column in step with the live inventory. */
watch(
  () => devices.value,
  () => {
    const device = selected.value
    if (device === null) {
      return
    }
    const fresh = devices.value.find(
      entry => entry.server === device.server && entry.ieee === device.ieee,
    )
    if (fresh === undefined) {
      selected.value = null
      statusPanel.clear()
      return
    }
    selected.value = fresh
    showDetail()
  },
)

onMounted(async () => {
  await refresh()
  joinServer.value = joinable.value[0]?.name ?? ''
})

onBeforeUnmount(() => {
  statusPanel.clear()
})
</script>

<template>
  <div class="devices">
    <header class="fd-between devices__head">
      <div>
        <div class="fd-label">
          Devices
        </div>
        <h1 class="fd-panel__title">
          What is on the network
        </h1>
        <div
          class="fd-meta fd-muted"
          data-testid="devices-count"
        >
          {{ shown }} of {{ total }} shown
        </div>
      </div>

      <div
        v-if="joinable.length > 0"
        class="fd-cluster"
        data-testid="devices-permit-join"
      >
        <select
          v-if="joinable.length > 1"
          v-model="joinServer"
          class="fd-select"
          aria-label="Server to open"
        >
          <option
            v-for="entry in joinable"
            :key="entry.name"
            :value="entry.name"
          >
            {{ entry.name }}
          </option>
        </select>
        <button
          class="fd-btn fd-btn--primary fd-btn--sm"
          type="button"
          :class="{ 'is-busy': joining }"
          :disabled="joining"
          @click="permitJoin"
        >
          Open join window
        </button>
      </div>
    </header>

    <AsyncState
      :loading="loading"
      :error="failure"
      :empty="!loading && total === 0"
      empty-text="No device has been seen yet. Open a join window and pair one."
      @retry="refresh"
    >
      <section class="fd-panel">
        <div class="fd-panel__body fd-stack">
          <nav
            class="fd-cluster tabs"
            aria-label="Device types"
            data-testid="device-type-tabs"
          >
            <button
              class="fd-btn fd-btn--ghost fd-btn--sm"
              type="button"
              :aria-pressed="type === 'all'"
              data-testid="device-tab-all"
              :data-active="type === 'all'"
              @click="type = 'all'"
            >
              All <span class="fd-meta fd-muted">{{ total }}</span>
            </button>
            <button
              v-for="entry in DEVICE_TABS"
              :key="entry.key"
              class="fd-btn fd-btn--ghost fd-btn--sm"
              type="button"
              :aria-pressed="type === entry.key"
              :data-testid="`device-tab-${entry.key}`"
              :data-active="type === entry.key"
              :disabled="(counts[entry.key] ?? 0) === 0"
              @click="type = entry.key"
            >
              {{ entry.label }}
              <span class="fd-meta fd-muted">{{ counts[entry.key] ?? 0 }}</span>
            </button>
          </nav>

          <DeviceFilterBar
            v-model:search="search"
            v-model:server="server"
            v-model:supported="supported"
            :servers="servers"
          />

          <DataTable
            :columns="columns"
            :rows="rows"
            row-key="ieee"
            testid="device-table"
            caption="Every device the bridge knows"
          >
            <template #cell-friendly_name="{ row }">
              <button
                class="link"
                type="button"
                data-testid="device-open"
                @click="select(row as unknown as DeviceInfo)"
              >
                {{ (row as Record<string, unknown>).friendly_name }}
              </button>
              <div class="fd-meta fd-muted">
                {{ (row as Record<string, unknown>).server }}
              </div>
            </template>

            <template #cell-readout="{ row }">
              <span
                v-if="rowReadout(row).text !== ''"
                class="readout"
                data-testid="device-readout"
              >
                <span class="readout__value">{{ rowReadout(row).text }}</span>
                <span
                  v-if="rowReadout(row).age !== ''"
                  class="fd-meta fd-muted"
                >· {{ rowReadout(row).age }}</span>
                <span
                  v-if="rowReadout(row).warn !== ''"
                  class="readout__warn"
                  data-testid="device-battery-warn"
                >{{ rowReadout(row).warn }}</span>
              </span>
              <span
                v-else
                class="fd-meta fd-muted"
              >—</span>
            </template>

            <template #cell-device_type="{ row }">
              {{ (row as Record<string, unknown>).device_type }}
            </template>

            <template #cell-model="{ row }">
              <span class="fd-meta">{{ (row as Record<string, unknown>).model ?? '—' }}</span>
            </template>

            <template #cell-capabilities="{ row }">
              <CapabilityBadges
                :capabilities="(row as unknown as DeviceInfo).capabilities"
                :limit="3"
              />
            </template>

            <template #cell-availability="{ row }">
              <span
                class="fd-status"
                :class="(row as Record<string, unknown>).availability === 'online'
                  ? 'fd-status--ok'
                  : (row as Record<string, unknown>).availability === 'offline'
                    ? 'fd-status--crit'
                    : 'fd-status--warn'"
              >{{ (row as Record<string, unknown>).availability }}</span>
            </template>

            <template #cell-mapping="{ row }">
              <span
                v-if="(row as Record<string, unknown>).supported"
                class="fd-meta"
                data-mapping="mapped"
              >mapped</span>
              <button
                v-else
                class="link link--warn"
                type="button"
                data-testid="device-why"
                @click="select(row as unknown as DeviceInfo)"
              >
                not mapped · Why?
              </button>
            </template>
          </DataTable>
        </div>
      </section>
    </AsyncState>
  </div>
</template>

<style scoped>
.devices {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.devices__head {
  flex-wrap: wrap;
  gap: var(--fd-2);
}
.tabs {
  gap: var(--fd-1);
  flex-wrap: wrap;
}
.tabs [data-active='true'] {
  background: var(--fd-accent-soft);
  color: var(--fd-accent-text);
}
.link {
  background: none;
  border: 0;
  padding: 0;
  font: inherit;
  color: var(--fd-text);
  text-align: left;
  cursor: pointer;
  text-decoration: underline;
  text-decoration-color: var(--fd-line-strong);
  text-underline-offset: 2px;
}
.link:hover {
  color: var(--fd-accent-text);
}
.link--warn {
  color: var(--fd-warn);
}
.readout {
  display: inline-flex;
  align-items: baseline;
  gap: var(--fd-1);
  flex-wrap: wrap;
}
.readout__value {
  font-variant-numeric: tabular-nums;
}
.readout__warn {
  color: var(--fd-warn);
  font-size: var(--fd-size-sm);
}
</style>
