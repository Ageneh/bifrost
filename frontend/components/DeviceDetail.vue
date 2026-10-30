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
import type { DeviceInfo, ServerInfo } from '~/lib/generated'

const props = defineProps<{
  device: DeviceInfo
  server?: ServerInfo
}>()

const emit = defineEmits<{
  identify: [device: DeviceInfo]
  close: []
}>()

const identifying = ref(false)
const identified = ref(false)

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
</style>
