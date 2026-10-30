<script setup lang="ts">
/**
 * A machine state in a word. The colour carries the state; the text carries
 * the meaning, so the pill still reads without colour.
 */
export type StatusState = 'ok' | 'warn' | 'crit' | 'live' | 'idle'

withDefaults(
  defineProps<{
    label: string
    state?: StatusState
    testid?: string
  }>(),
  { state: 'idle', testid: 'status-pill' },
)
</script>

<template>
  <span
    class="fd-status"
    :class="`fd-status--${state}`"
    :data-testid="testid"
    :data-state="state"
  >
    {{ label }}
  </span>
</template>
