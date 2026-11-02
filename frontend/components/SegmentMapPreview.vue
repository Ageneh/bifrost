<script setup lang="ts">
/**
 * The zone as a strip: where each channel sits across the screen.
 *
 * It is the one picture that answers "did I put the left light on the left",
 * and it is drawn from the same channels the table lists. The colour of a
 * swatch is only its channel id, not a colour the TV will show: a frame
 * decides that.
 */
import { computed } from 'vue'
import type { EntertainmentZoneChannel } from '~/lib/generated'

const props = defineProps<{
  channels: EntertainmentZoneChannel[]
  testid?: string
}>()

/** Channels left to right, which is how a screen reads. */
const ordered = computed(() => [...props.channels].sort(
  (a, b) => a.position.x - b.position.x || a.channel_id - b.channel_id,
))

function left(x: number): string {
  return `${Math.min(Math.max((x + 1) / 2, 0), 1) * 100}%`
}
</script>

<template>
  <div
    class="fd-stack"
    :data-testid="testid ?? 'segment-preview'"
  >
    <div class="strip">
      <span
        v-for="channel in ordered"
        :key="channel.channel_id"
        class="strip__swatch"
        :data-testid="`segment-swatch-${channel.channel_id}`"
        :style="{ left: left(channel.position.x) }"
        :title="`channel ${channel.channel_id}`"
      >
        {{ channel.channel_id }}
      </span>
    </div>
    <div class="fd-meta fd-muted">
      Left to right by x. {{ channels.length }} channel{{ channels.length === 1 ? '' : 's' }},
      one colour each, in channel order.
    </div>
  </div>
</template>

<style scoped>
.strip {
  position: relative;
  height: 44px;
  border: 1px solid var(--fd-line-strong);
  border-radius: var(--fd-r-inset);
  background: var(--fd-surface-1);
}
.strip__swatch {
  position: absolute;
  top: 50%;
  transform: translate(-50%, -50%);
  width: 22px;
  height: 22px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  border: 1px solid var(--fd-accent-line);
  background: var(--fd-accent-soft);
  color: var(--fd-accent-text);
  font-size: var(--fd-size-micro);
  font-variant-numeric: tabular-nums;
}
</style>
