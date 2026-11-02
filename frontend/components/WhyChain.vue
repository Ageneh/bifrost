<script setup lang="ts">
/**
 * Device → z2m → group → room, with the first broken hop in the critical
 * colour.
 *
 * This is the answer to "my bulb is missing from the Hue app": every hop is
 * shown, the ones that hold are quiet, and exactly one is loud — the first
 * one that does not hold. Everything after it is unknown rather than broken,
 * because it cannot be reached from a hop that already failed.
 */
import { computed } from 'vue'
import type { DeviceInfo, ServerInfo } from '~/lib/generated'

const props = defineProps<{
  device: DeviceInfo
  server?: ServerInfo
}>()

interface Hop {
  key: string
  label: string
  detail: string
  state: 'ok' | 'broken' | 'unknown'
}

const hops = computed<Hop[]>(() => {
  const device = props.device
  const server = props.server

  const serverHop: Omit<Hop, 'state'> = {
    key: 'z2m',
    label: 'z2m',
    detail: server
      ? `${device.server} · ${server.online ? 'connected' : 'not connected'}`
      : `${device.server} · not running`,
  }
  const groupHop: Omit<Hop, 'state'> = {
    key: 'group',
    label: 'group',
    detail: device.group ? `${device.group.name} (${device.group.id})` : 'no group',
  }
  const roomHop: Omit<Hop, 'state'> = {
    key: 'room',
    label: 'room',
    detail: device.room ? device.room.name : 'no room',
  }

  const chain: Hop[] = [
    { key: 'device', label: 'device', detail: device.friendly_name, state: 'ok' },
  ]

  // the first hop that does not hold is the one worth naming; the rest are
  // unknown, not broken
  let broken = false
  const push = (hop: Omit<Hop, 'state'>, holds: boolean) => {
    if (broken) {
      chain.push({ ...hop, state: 'unknown' })
      return
    }
    chain.push({ ...hop, state: holds ? 'ok' : 'broken' })
    broken = !holds
  }

  push(serverHop, server?.online === true)
  push(groupHop, device.group !== null)
  push(roomHop, device.room !== null)

  return chain
})

/** The first hop that does not hold, when there is one. */
const brokenHop = computed(() => hops.value.find(hop => hop.state === 'broken'))
</script>

<template>
  <div
    class="chain"
    data-testid="why-chain"
    :data-broken-hop="brokenHop?.key ?? ''"
  >
    <template
      v-for="(hop, index) in hops"
      :key="hop.key"
    >
      <span
        v-if="index > 0"
        class="chain__link"
        aria-hidden="true"
      >→</span>
      <span
        class="chain__hop"
        :data-hop="hop.key"
        :data-state="hop.state"
        :data-testid="`why-hop-${hop.key}`"
      >
        <span class="chain__label">{{ hop.label }}</span>
        <span class="chain__detail">{{ hop.detail }}</span>
      </span>
    </template>
  </div>
</template>

<style scoped>
.chain {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--fd-1);
}
.chain__link {
  color: var(--fd-text-3);
  font-size: var(--fd-size-meta);
}
.chain__hop {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: var(--fd-1) var(--fd-2);
  border: 1px solid var(--fd-line);
  border-radius: var(--fd-r-inset);
  background: var(--fd-surface-2);
  min-width: 0;
}
.chain__hop[data-state='broken'] {
  border-color: var(--fd-crit);
  background: var(--fd-crit-soft);
}
.chain__hop[data-state='broken'] .chain__detail {
  color: var(--fd-crit-text);
}
.chain__hop[data-state='unknown'] {
  opacity: 0.55;
}
.chain__label {
  font-size: var(--fd-size-micro);
  letter-spacing: var(--fd-track-micro);
  text-transform: uppercase;
  color: var(--fd-text-3);
}
.chain__detail {
  font-size: var(--fd-size-sm);
  color: var(--fd-text);
}
</style>
