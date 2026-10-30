<script setup lang="ts">
/**
 * The link button, and the window it opens.
 *
 * A real bridge has a physical button that pairs for thirty seconds. This is
 * the stand-in: one large button, a countdown while the window is open, and
 * a plain statement that the window grants nothing the open Hue API does not
 * already grant. It is a realism tool, not a lock.
 */
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    pressed: boolean
    /** Whole seconds left in the window. */
    remaining: number
    /** The window the button asks for. */
    windowSeconds?: number
    busy?: boolean
    /** Refused because the bridge demands a window and none is open. */
    disabledReason?: string
  }>(),
  { windowSeconds: 30, busy: false, disabledReason: '' },
)

const emit = defineEmits<{ press: [] }>()

/** How much of the window is left, as a percentage. */
const fill = computed(() => {
  if (!props.pressed || props.windowSeconds <= 0) {
    return 0
  }
  return Math.max(0, Math.min(100, Math.round((props.remaining / props.windowSeconds) * 100)))
})

const label = computed(() => (props.pressed ? `${props.remaining}s` : 'Open pairing window'))
</script>

<template>
  <section
    class="fd-panel"
    data-testid="link-button-card"
    :data-state="pressed ? 'open' : 'closed'"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          Link button
        </div>
        <div class="fd-panel__sub">
          Open a window, then pair from the client. A window stays open for
          {{ windowSeconds }} seconds unless another duration is chosen.
        </div>
      </div>
      <span
        class="fd-badge"
        :data-state="pressed ? 'ok' : 'idle'"
        data-testid="link-button-state"
      >
        {{ pressed ? 'window open' : 'closed' }}
      </span>
    </div>

    <div
      class="fd-cluster"
      style="gap: var(--fd-4); align-items: center"
    >
      <button
        type="button"
        class="fd-btn fd-btn--primary"
        style="min-height: 64px; min-width: 220px; font-size: 1.1rem"
        :disabled="busy"
        data-testid="link-button-press"
        @click="emit('press')"
      >
        {{ label }}
      </button>

      <div
        class="fd-meter fd-meter--ok"
        style="flex: 1; min-width: 120px"
        role="progressbar"
        :aria-valuenow="remaining"
        :aria-valuemin="0"
        :aria-valuemax="windowSeconds"
        aria-label="Pairing window"
        data-testid="link-button-meter"
      >
        <div
          class="fd-meter__fill"
          :style="{ width: `${fill}%` }"
        />
      </div>
    </div>

    <p
      class="fd-muted"
      style="margin-top: var(--fd-3)"
      data-testid="link-button-honesty"
    >
      Pairing grants nothing an open Hue API does not: every client receives the same fixed
      username and the same DTLS key, whether or not a window is open.
    </p>

    <p
      v-if="disabledReason"
      class="fd-muted"
      data-testid="link-button-disabled"
    >
      {{ disabledReason }}
    </p>
  </section>
</template>
