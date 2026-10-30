<script setup lang="ts">
/**
 * Every mode that has no screen yet.
 *
 * The navigation keeps all six modes visible from the first wave, so a
 * missing screen says so in place instead of disappearing from the rail.
 */
import { computed } from 'vue'
import { useRoute } from 'vue-router'

const route = useRoute()

const TITLES: Record<string, string> = {
  devices: 'Devices',
  rooms: 'Rooms',
  entertainment: 'Entertainment',
  pairing: 'Pairing',
  system: 'System',
}

const key = computed(() => String(route.params.slug ?? '').replace(/^\/+/, ''))
const title = computed(() => TITLES[key.value] ?? 'Not found')
</script>

<template>
  <div
    class="fd-panel"
    data-testid="pending-screen"
    :data-state="TITLES[key] ? 'pending' : 'missing'"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          {{ title }}
        </div>
        <div class="fd-panel__sub">
          <template v-if="TITLES[key]">
            This screen is not built yet. It arrives in a later wave; the shell, the draft bar and
            the live connection already work.
          </template>
          <template v-else>
            No mode lives at <span class="mono">#/{{ key }}</span>.
          </template>
        </div>
      </div>
      <NuxtLink
        to="/"
        class="fd-btn fd-btn--sm"
        data-testid="pending-home"
      >Back to Home</NuxtLink>
    </div>
  </div>
</template>
