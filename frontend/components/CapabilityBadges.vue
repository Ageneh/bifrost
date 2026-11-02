<script setup lang="ts">
/**
 * What a device can do, as chips.
 *
 * Only the capabilities that are true are shown: an absent badge means the
 * device cannot do it, which is the reading a person makes anyway. A device
 * with no capability at all says so rather than showing an empty row.
 */
import { computed } from 'vue'
import type { Capabilities } from '~/lib/generated'

const props = withDefaults(
  defineProps<{
    capabilities: Capabilities
    /** Show at most this many, and count the rest. */
    limit?: number
  }>(),
  { limit: 4 },
)

interface Badge {
  key: keyof Capabilities
  label: string
}

/** The order is by how much a person cares, not alphabetical. */
const ALL: Badge[] = [
  { key: 'dimmable', label: 'dim' },
  { key: 'color', label: 'color' },
  { key: 'color_temperature', label: 'temp' },
  { key: 'gradient', label: 'gradient' },
  { key: 'effects', label: 'effects' },
  { key: 'motion', label: 'motion' },
  { key: 'temperature', label: 'temperature' },
  { key: 'illuminance', label: 'lux' },
  { key: 'contact', label: 'contact' },
  { key: 'action', label: 'button' },
]

const shown = computed(() => ALL.filter(badge => props.capabilities[badge.key]))
const visible = computed(() => shown.value.slice(0, props.limit))
const hidden = computed(() => Math.max(0, shown.value.length - props.limit))
</script>

<template>
  <span
    class="fd-cluster capabilities"
    data-testid="capability-badges"
  >
    <span
      v-for="badge in visible"
      :key="badge.key"
      class="fd-chip"
      :data-capability="badge.key"
    >{{ badge.label }}</span>
    <span
      v-if="hidden > 0"
      class="fd-meta fd-muted"
    >+{{ hidden }}</span>
    <span
      v-if="shown.length === 0"
      class="fd-meta fd-muted"
    >—</span>
  </span>
</template>

<style scoped>
.capabilities {
  gap: var(--fd-1);
  flex-wrap: wrap;
}
</style>
