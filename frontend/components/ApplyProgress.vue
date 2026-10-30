<script setup lang="ts">
/**
 * The apply, while it is happening.
 *
 * A persistent bar across the top, never a modal: an apply can restart a
 * listener and take a while, and a modal would block the console the user
 * may still need. It also carries the reconnect case, where the socket
 * dropped mid-apply and the result is being reconciled.
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useDraft } from '~/composables/useDraft'

const draft = useDraft()
const elapsed = ref(0)
let timer: ReturnType<typeof setInterval> | null = null

const visible = computed(
  () => draft.status.value === 'applying' || draft.applyAttempt.value !== null,
)

const text = computed(() => {
  if (draft.status.value === 'applying') {
    return `Applying revision ${draft.applyAttempt.value?.revision ?? '—'}`
  }
  return 'Reconciling the apply after a reconnect'
})

watch(
  visible,
  (shown) => {
    if (shown && timer === null) {
      elapsed.value = 0
      timer = setInterval(() => {
        elapsed.value += 1
      }, 1000)
    }
    else if (!shown && timer !== null) {
      clearInterval(timer)
      timer = null
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  if (timer !== null) {
    clearInterval(timer)
  }
})
</script>

<template>
  <div
    v-if="visible"
    class="progress"
    data-testid="apply-progress"
    :data-state="draft.status.value"
    role="status"
    aria-live="polite"
  >
    <span class="fd-status fd-status--live">apply</span>
    <span class="fd-grow">{{ text }}</span>
    <span class="fd-meta fd-num">{{ elapsed }}s</span>
    <span
      v-if="draft.applyAttempt.value"
      class="fd-meta mono"
      data-testid="apply-id"
    >
      {{ draft.applyAttempt.value.applyId.slice(0, 8) }}
    </span>
  </div>
</template>

<style scoped>
.progress {
  position: sticky;
  top: 0;
  z-index: 55;
  display: flex;
  align-items: center;
  gap: var(--fd-3);
  padding: var(--fd-2) var(--fd-4);
  background: var(--fd-surface-2);
  border-bottom: 1px solid var(--fd-accent-line);
}
</style>
