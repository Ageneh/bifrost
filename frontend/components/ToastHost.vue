<script setup lang="ts">
/**
 * The toast stack. Toasts are announcements, never the only place a result
 * is shown: the panel that caused one keeps its own state.
 */
import { useToast } from '~/composables/useToast'

const { toasts, dismiss } = useToast()
</script>

<template>
  <div
    class="fd-toasts"
    data-testid="toast-host"
    aria-live="polite"
  >
    <div
      v-for="toast in toasts"
      :key="toast.id"
      class="fd-toast is-entering"
      :class="`fd-toast--${toast.kind}`"
      :data-testid="`toast-${toast.kind}`"
      :role="toast.kind === 'crit' ? 'alert' : 'status'"
    >
      <span
        class="fd-toast__bar"
        aria-hidden="true"
      />
      <span class="fd-grow">{{ toast.message }}</span>
      <button
        type="button"
        class="fd-btn fd-btn--ghost fd-btn--sm"
        :data-testid="`toast-dismiss-${toast.id}`"
        @click="dismiss(toast.id)"
      >
        Dismiss
      </button>
    </div>
  </div>
</template>
