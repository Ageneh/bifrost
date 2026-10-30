<script setup lang="ts">
/**
 * What is streaming right now, in the status column.
 *
 * One row per area, with the two rates that matter and the stop that ends
 * it. There is no start: the TV or the Sync app begins a stream by sending
 * its first frame, and a button here could only announce frames no client
 * will send.
 */
import type { StreamStats } from '~/lib/generated'

defineProps<{
  streams: StreamStats[]
  /** The name of the zone a stream belongs to, by area. */
  nameOf: (area: string) => string
  maxStreams: number
  failure?: string | null
  testid?: string
}>()

defineEmits<{ stop: [area: string] }>()
</script>

<template>
  <section
    class="fd-panel"
    :data-testid="testid ?? 'stream-status'"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          Streaming
        </div>
        <div class="fd-panel__sub">
          {{ streams.length }} of {{ maxStreams }} allowed. The TV or the app
          starts a stream; this ends one.
        </div>
      </div>
      <span
        class="fd-status"
        :class="streams.length > 0 ? 'fd-status--live' : 'fd-status--ok'"
        :data-testid="`${testid ?? 'stream-status'}-state`"
      >{{ streams.length > 0 ? 'live' : 'idle' }}</span>
    </div>

    <div class="fd-panel__body fd-stack">
      <div
        v-if="failure"
        class="fd-alert"
        :data-testid="`${testid ?? 'stream-status'}-failure`"
      >
        {{ failure }}
      </div>

      <div
        v-if="streams.length === 0 && !failure"
        class="fd-muted"
        :data-testid="`${testid ?? 'stream-status'}-empty`"
      >
        Nothing is streaming. Start it from the TV or the Sync app.
      </div>

      <div
        v-for="stream in streams"
        :key="stream.area"
        class="fd-between stream"
        :data-testid="`${testid ?? 'stream-status'}-row-${stream.area}`"
      >
        <div>
          <div class="fd-value">
            {{ nameOf(stream.area) }}
          </div>
          <div class="fd-meta fd-muted">
            <span class="fd-num">{{ stream.fps_in }} in / {{ stream.fps_out }} out fps</span>
            · <span class="fd-num">{{ stream.drops }} dropped</span>
          </div>
        </div>
        <button
          type="button"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          :data-testid="`${testid ?? 'stream-status'}-stop-${stream.area}`"
          @click="$emit('stop', stream.area)"
        >
          Stop
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.stream {
  align-items: flex-start;
}
</style>
