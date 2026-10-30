<script setup lang="ts">
/**
 * The one way to choose devices in the console.
 *
 * A table with a checkbox per row, narrowed by type and by a search over
 * name, model and address, with the capabilities each device offers. A
 * device Bifrost cannot serve is shown but not selectable, and says why:
 * a room cannot hold what the Hue API does not know.
 *
 * No screen grows its own picker; this one is opened with the devices it
 * should leave out and the ones already chosen.
 */
import { computed, nextTick, ref, watch } from 'vue'
import CapabilityBadges from '~/components/CapabilityBadges.vue'
import { DEVICE_TYPES, useInventory } from '~/composables/useInventory'
import type { DeviceInfo, DeviceType } from '~/lib/generated'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    /** Devices the caller already holds, by IEEE address; shown ticked. */
    selected?: string[]
    /** Devices that may not be chosen, e.g. those already elsewhere. */
    exclude?: string[]
    /** Only this class is offered; absent offers every class. */
    only?: DeviceType | null
    testid?: string
  }>(),
  {
    title: 'Choose devices',
    selected: () => [],
    exclude: () => [],
    only: null,
    testid: 'device-picker',
  },
)

const emit = defineEmits<{ close: [], confirm: [ieees: string[]] }>()

const { devices, loading, refresh } = useInventory()

const type = ref<DeviceType | 'all'>(props.only ?? 'all')
const search = ref('')
const chosen = ref<string[]>([])

const columns = [
  { key: 'pick', label: '' },
  { key: 'friendly_name', label: 'Device' },
  { key: 'device_type', label: 'Type' },
  { key: 'capabilities', label: 'Capabilities' },
]

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      return
    }
    chosen.value = [...props.selected]
    search.value = ''
    type.value = props.only ?? 'all'
    if (devices.value.length === 0) {
      await refresh()
    }
    await nextTick()
  },
)

/** The rows the tabs, the search and the exclusion leave. */
const rows = computed(() => {
  const needle = search.value.trim().toLowerCase()

  return devices.value
    .filter(device => !props.exclude.includes(device.ieee))
    .filter(device => type.value === 'all' || device.device_type === type.value)
    .filter((device) => {
      if (needle === '') {
        return true
      }
      return [device.friendly_name, device.model, device.vendor, device.ieee]
        .some(value => (value ?? '').toLowerCase().includes(needle))
    })
})

/** How many devices each tab holds, after the exclusion. */
const counts = computed<Record<string, number>>(() => {
  const tally: Record<string, number> = {}
  for (const device of devices.value) {
    if (props.exclude.includes(device.ieee)) {
      continue
    }
    tally[device.device_type] = (tally[device.device_type] ?? 0) + 1
  }
  return tally
})

function toggle(device: DeviceInfo, on: boolean): void {
  const without = chosen.value.filter(ieee => ieee !== device.ieee)
  chosen.value = on ? [...without, device.ieee] : without
}

function confirm(): void {
  emit('confirm', chosen.value)
}
</script>

<template>
  <div
    v-if="open"
    class="fd-scrim"
    :data-testid="testid"
    data-state="open"
    @keydown.esc="emit('close')"
  >
    <div
      class="fd-modal picker"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
    >
      <div class="fd-between">
        <div>
          <div class="fd-panel__title">
            {{ title }}
          </div>
          <div
            class="fd-meta fd-muted"
            :data-testid="`${testid}-count`"
          >
            {{ chosen.length }} chosen
          </div>
        </div>
        <button
          type="button"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          :data-testid="`${testid}-close`"
          @click="emit('close')"
        >
          Close
        </button>
      </div>

      <nav
        class="fd-cluster tabs"
        aria-label="Device types"
        :data-testid="`${testid}-tabs`"
      >
        <button
          v-if="only === null"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          type="button"
          :aria-pressed="type === 'all'"
          :data-active="type === 'all'"
          :data-testid="`${testid}-tab-all`"
          @click="type = 'all'"
        >
          All <span class="fd-meta fd-muted">{{ rows.length }}</span>
        </button>
        <button
          v-for="entry in DEVICE_TYPES"
          :key="entry.key"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          type="button"
          :aria-pressed="type === entry.key"
          :data-active="type === entry.key"
          :data-testid="`${testid}-tab-${entry.key}`"
          :disabled="(counts[entry.key] ?? 0) === 0"
          @click="type = entry.key"
        >
          {{ entry.label }}
          <span class="fd-meta fd-muted">{{ counts[entry.key] ?? 0 }}</span>
        </button>
      </nav>

      <label class="fd-field">
        <span class="fd-label">Search</span>
        <input
          v-model="search"
          class="fd-input"
          type="search"
          placeholder="Name, model or address"
          :data-testid="`${testid}-search`"
        >
      </label>

      <div class="picker__body">
        <table
          class="fd-table"
          data-layout="table"
          :data-testid="`${testid}-table`"
        >
          <thead>
            <tr>
              <th
                v-for="column in columns"
                :key="column.key"
                scope="col"
              >
                {{ column.label }}
              </th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="device in rows"
              :key="device.ieee"
              :data-testid="`${testid}-row`"
              :data-supported="device.supported"
            >
              <td data-label="">
                <input
                  type="checkbox"
                  :checked="chosen.includes(device.ieee)"
                  :disabled="!device.supported"
                  :aria-label="device.friendly_name"
                  :data-testid="`${testid}-pick`"
                  @change="toggle(device, ($event.target as HTMLInputElement).checked)"
                >
              </td>
              <td data-label="Device">
                {{ device.friendly_name }}
                <div
                  v-if="!device.supported"
                  class="fd-meta fd-muted"
                  :data-testid="`${testid}-unsupported`"
                >
                  {{ device.unsupported_reason ?? 'Bifrost cannot serve this device' }}
                </div>
              </td>
              <td data-label="Type">
                {{ device.device_type }}
              </td>
              <td data-label="Capabilities">
                <CapabilityBadges
                  :capabilities="device.capabilities"
                  :limit="3"
                />
              </td>
            </tr>
          </tbody>
        </table>

        <div
          v-if="!loading && rows.length === 0"
          class="fd-muted"
          :data-testid="`${testid}-empty`"
        >
          No device matches.
        </div>
      </div>

      <div class="fd-modal__actions">
        <button
          type="button"
          class="fd-btn fd-btn--ghost"
          :data-testid="`${testid}-cancel`"
          @click="emit('close')"
        >
          Cancel
        </button>
        <button
          type="button"
          class="fd-btn fd-btn--primary"
          :data-testid="`${testid}-confirm`"
          @click="confirm"
        >
          Use {{ chosen.length }} device{{ chosen.length === 1 ? '' : 's' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.picker {
  width: min(760px, 100%);
  max-height: 86vh;
  display: flex;
  flex-direction: column;
  gap: var(--fd-2);
}
.picker__body {
  overflow: auto;
  min-height: 0;
}
.tabs {
  gap: var(--fd-1);
  flex-wrap: wrap;
}
.tabs [data-active='true'] {
  background: var(--fd-accent-soft);
  color: var(--fd-accent-text);
}
</style>
