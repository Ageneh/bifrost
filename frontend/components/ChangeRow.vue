<script setup lang="ts">
/**
 * One changed setting: what it was, what it will be.
 *
 * A secret never shows its values; it shows that it changes, because the
 * server redacts both sides and the user still needs to know it moved.
 */
import { computed } from 'vue'
import type { ConfigChange } from '~/lib/generated/ConfigChange'

const props = defineProps<{ change: ConfigChange }>()

const before = computed(() => (props.change.before === null ? '—' : render(props.change.before)))
const after = computed(() => (props.change.after === null ? '—' : render(props.change.after)))

function render(value: unknown): string {
  if (typeof value === 'string') {
    return value
  }
  return JSON.stringify(value)
}
</script>

<template>
  <div
    class="fd-row__top"
    data-testid="change-row"
    :data-state="change.secret ? 'secret' : 'plain'"
  >
    <div
      class="fd-grow"
      style="min-width: 0"
    >
      <div class="fd-row__path">
        {{ change.path }}
      </div>
      <div
        class="fd-meta"
        style="margin-top: var(--fd-1)"
      >
        <template v-if="change.secret">
          Secret changes; the value stays on the bridge.
        </template>
        <template v-else>
          <span class="fd-muted">{{ before }}</span>
          <span aria-hidden="true"> → </span>
          <span>{{ after }}</span>
        </template>
      </div>
    </div>
    <span
      v-if="change.secret"
      class="fd-chip"
    >secret</span>
  </div>
</template>
