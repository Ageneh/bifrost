<script setup lang="ts">
/**
 * One device, in the status column: what it is, what it can do, whether
 * Bifrost serves it, and the chain that explains a device that is missing
 * from the Hue app.
 *
 * The only action is identify, which flashes the device. This wave is not a
 * control surface: no on/off, no brightness.
 */
import { computed, ref } from 'vue'
import CapabilityBadges from '~/components/CapabilityBadges.vue'
import WhyChain from '~/components/WhyChain.vue'
import { LOW_BATTERY, useInventory } from '~/composables/useInventory'
import type { DeviceEvent, DeviceInfo, ServerInfo } from '~/lib/generated'

const props = defineProps<{
  device: DeviceInfo
  server?: ServerInfo
}>()

const inventory = useInventory()

const emit = defineEmits<{
  identify: [device: DeviceInfo]
  close: []
}>()

const identifying = ref(false)
const identified = ref(false)

/** What this device reported while the page was open, newest first. */
const events = computed<DeviceEvent[]>(() => inventory.eventsFor(props.device.ieee).slice(0, 12))

/** A remote's last press, as the chip the row also shows. */
const lastAction = computed(() => inventory.lastAction(props.device))

/** The battery level, when the device reports one. */
const battery = computed(() => props.device.battery)

const batteryLow = computed(
  () => battery.value !== null && battery.value < LOW_BATTERY,
)

/** An event's value as one line of text. */
function describe(event: DeviceEvent): string {
  const value = event.value
  if (typeof value === 'boolean') {
    if (event.kind === 'contact') {
      return value ? 'open' : 'closed'
    }
    if (event.kind === 'occupancy') {
      return value ? 'motion' : 'clear'
    }
    return value ? 'yes' : 'no'
  }
  if (typeof value === 'number') {
    if (event.kind === 'temperature') {
      return `${value.toFixed(1)} °C`
    }
    if (event.kind === 'illuminance') {
      return `${Math.round(value)} lx`
    }
    return String(value)
  }
  return String(value)
}

/** An event's time, as the clock a person reads. */
function clock(at: string): string {
  const date = new Date(at)
  if (Number.isNaN(date.getTime())) {
    return at
  }
  return date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

const availability = computed(() => {
  if (props.device.availability === 'online') {
    return { label: 'online', state: 'ok' as const }
  }
  if (props.device.availability === 'offline') {
    return { label: 'offline', state: 'crit' as const }
  }
  return { label: 'unknown', state: 'warn' as const }
})

async function identify(): Promise<void> {
  identifying.value = true
  identified.value = false
  try {
    emit('identify', props.device)
    identified.value = true
  }
  finally {
    identifying.value = false
  }
}
</script>

<template>
  <section
    class="fd-panel"
    data-testid="device-detail"
    :data-ieee="device.ieee"
  >
    <header class="fd-panel__head">
      <div class="fd-between">
        <div>
          <div class="fd-label">
            Device
          </div>
          <h2 class="fd-panel__title">
            {{ device.friendly_name }}
          </h2>
        </div>
        <button
          class="fd-btn fd-btn--ghost fd-btn--sm"
          type="button"
          aria-label="Close the device"
          data-testid="device-detail-close"
          @click="emit('close')"
        >
          Close
        </button>
      </div>
      <div class="fd-panel__sub">
        {{ device.vendor ?? 'unknown vendor' }} · {{ device.model ?? 'unknown model' }} ·
        {{ device.server }}
      </div>
    </header>

    <div class="fd-panel__body fd-stack">
      <div class="fd-cluster">
        <span
          class="fd-status"
          :class="`fd-status--${availability.state}`"
          data-testid="device-availability"
        >{{ availability.label }}</span>
        <span
          class="fd-chip"
          :data-mapped="device.supported ? 'yes' : 'no'"
          data-testid="device-mapping"
        >{{ device.supported ? 'mapped' : 'not mapped' }}</span>
      </div>

      <div
        v-if="device.device_type === 'remote'"
        class="fd-cluster"
        data-testid="device-last-action"
      >
        <span class="fd-label">Last press</span>
        <span
          class="fd-chip"
          data-testid="device-action-chip"
        >{{ lastAction ?? 'none yet' }}</span>
      </div>

      <div
        v-if="battery !== null"
        class="fd-cluster"
        data-testid="device-battery"
        :data-low="batteryLow ? 'true' : 'false'"
      >
        <span class="fd-label">Battery</span>
        <span
          class="fd-status"
          :class="batteryLow ? 'fd-status--warn' : 'fd-status--ok'"
        >{{ battery }} %</span>
      </div>

      <p
        v-if="device.unsupported_reason"
        class="fd-meta fd-muted"
        data-testid="device-unsupported-reason"
      >
        {{ device.unsupported_reason }}
      </p>

      <div>
        <div class="fd-label">
          Capabilities
        </div>
        <CapabilityBadges
          :capabilities="device.capabilities"
          :limit="10"
        />
      </div>

      <div>
        <div class="fd-label">
          Why it is here, or why it is not
        </div>
        <WhyChain
          :device="device"
          :server="server"
        />
      </div>

      <div>
        <div class="fd-label">
          Address
        </div>
        <div class="fd-value fd-num">
          {{ device.ieee }}
        </div>
      </div>

      <div>
        <div class="fd-label">
          Recent events
        </div>
        <ul
          v-if="events.length > 0"
          class="events"
          data-testid="device-events"
        >
          <li
            v-for="(event, index) in events"
            :key="`${event.at}-${index}`"
            class="fd-meta"
          >
            <span class="fd-num fd-muted">{{ clock(event.at) }}</span>
            <span> · {{ event.kind }}</span>
            <span class="fd-num"> · {{ describe(event) }}</span>
          </li>
        </ul>
        <p
          v-else
          class="fd-meta fd-muted"
          data-testid="device-events-empty"
        >
          Nothing reported since this page opened.
        </p>
      </div>

      <div>
        <div class="fd-label">
          Exposes
        </div>
        <ul class="exposes">
          <li
            v-for="expose in device.exposes"
            :key="expose.property"
            class="fd-meta"
          >
            <span class="fd-num">{{ expose.property }}</span>
            <span class="fd-muted"> · {{ expose.kind }}{{ expose.unit ? ` · ${expose.unit}` : '' }}</span>
          </li>
        </ul>
      </div>

      <div class="fd-cluster">
        <button
          class="fd-btn fd-btn--sm"
          type="button"
          data-testid="device-identify"
          :class="{ 'is-busy': identifying }"
          :disabled="identifying"
          @click="identify"
        >
          Identify
        </button>
        <span
          v-if="identified"
          class="fd-meta fd-muted"
          data-testid="device-identified"
        >flashing now</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.exposes {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 160px;
  overflow: auto;
}
.exposes li {
  padding: 1px 0;
}
.events {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 160px;
  overflow: auto;
}
.events li {
  padding: 1px 0;
  font-variant-numeric: tabular-nums;
}
</style>
