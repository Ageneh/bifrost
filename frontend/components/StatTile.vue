<script setup lang="ts">
/**
 * One number with its label and unit. Numbers are tabular so a value that
 * changes keeps its width and the row does not jump.
 */
withDefaults(
  defineProps<{
    label: string
    value: string | number
    unit?: string
    hint?: string
    state?: 'ok' | 'warn' | 'crit' | 'idle'
    testid?: string
  }>(),
  { state: 'idle', testid: 'stat-tile' },
)
</script>

<template>
  <div
    class="fd-readout"
    :data-testid="testid"
    :data-state="state"
  >
    <span class="fd-label">{{ label }}</span>
    <span class="fd-value fd-num">
      {{ value }}<span
        v-if="unit"
        class="fd-unit"
      >{{ unit }}</span>
    </span>
    <span
      v-if="hint"
      class="fd-meta"
    >{{ hint }}</span>
  </div>
</template>
