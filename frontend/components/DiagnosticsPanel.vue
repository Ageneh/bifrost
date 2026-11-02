<script setup lang="ts">
/**
 * Why a stream is not running, read-only.
 *
 * The numbers are the server's: the session counts frames as they arrive,
 * the backend counts what the area's throttle dropped, and both are sampled
 * once a second. A row stays after its stream ends when it ended badly, so
 * the panel can say what happened rather than only that nothing is running.
 */
import { computed } from 'vue'
import type { StreamStats } from '~/lib/generated'

const props = defineProps<{
  streams: StreamStats[]
  nameOf: (area: string) => string
  testid?: string
}>()

/** A stream that ended with a reason, which is what this panel exists for. */
const ended = computed(() => props.streams.filter(stream => stream.last_error !== null))

function started(stream: StreamStats): string {
  const at = new Date(stream.started)
  return Number.isNaN(at.valueOf()) ? stream.started : at.toLocaleTimeString()
}
</script>

<template>
  <section
    class="fd-panel"
    :data-testid="testid ?? 'diagnostics'"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          Diagnostics
        </div>
        <div class="fd-panel__sub">
          Read-only, sampled once a second. In is what the sender delivered;
          out is what reached zigbee2mqtt after the area's throttle.
        </div>
      </div>
    </div>

    <div class="fd-panel__body fd-stack">
      <div
        v-if="ended.length > 0"
        class="fd-alert"
        data-testid="diagnostics-errors"
      >
        <div
          v-for="stream in ended"
          :key="`error-${stream.area}`"
          :data-testid="`diagnostics-error-${stream.area}`"
        >
          {{ nameOf(stream.area) }}: {{ stream.last_error }}
        </div>
      </div>

      <table
        v-if="streams.length > 0"
        class="fd-table"
        data-layout="cards"
        data-testid="diagnostics-table"
      >
        <thead>
          <tr>
            <th scope="col">
              Area
            </th>
            <th scope="col">
              In
            </th>
            <th scope="col">
              Out
            </th>
            <th scope="col">
              Frames
            </th>
            <th scope="col">
              Drops
            </th>
            <th scope="col">
              Since
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="stream in streams"
            :key="stream.area"
            :data-testid="`diagnostics-row-${stream.area}`"
          >
            <td data-label="Area">
              {{ nameOf(stream.area) }}
            </td>
            <td
              data-label="In"
              class="fd-num"
            >
              {{ stream.fps_in }}
            </td>
            <td
              data-label="Out"
              class="fd-num"
            >
              {{ stream.fps_out }}
            </td>
            <td
              data-label="Frames"
              class="fd-num"
            >
              {{ stream.frames }}
            </td>
            <td
              data-label="Drops"
              class="fd-num"
            >
              {{ stream.drops }}
            </td>
            <td data-label="Since">
              {{ started(stream) }}
            </td>
          </tr>
        </tbody>
      </table>

      <div
        v-else
        class="fd-muted"
        data-testid="diagnostics-empty"
      >
        No stream is running. There is nothing to diagnose until the TV or the
        Sync app starts one.
      </div>
    </div>
  </section>
</template>
