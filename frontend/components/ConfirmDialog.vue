<script setup lang="ts">
/**
 * The modal for an action that cannot be undone.
 *
 * Destructive confirmations can require the user to type a word first, so a
 * stray Enter on a focused button cannot wipe anything.
 */
import { computed, nextTick, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    title: string
    message?: string
    confirmLabel?: string
    cancelLabel?: string
    danger?: boolean
    /** When set, the user must type this exactly to confirm. */
    requireText?: string
    busy?: boolean
    testid?: string
  }>(),
  {
    message: '',
    confirmLabel: 'Confirm',
    cancelLabel: 'Cancel',
    danger: false,
    requireText: '',
    busy: false,
    testid: 'confirm-dialog',
  },
)

const emit = defineEmits<{ confirm: [], cancel: [] }>()

const typed = ref('')
const cancelButton = ref<HTMLButtonElement | null>(null)

const canConfirm = computed(
  () => !props.busy && (props.requireText === '' || typed.value === props.requireText),
)

watch(
  () => props.open,
  async (open) => {
    typed.value = ''
    if (open) {
      await nextTick()
      // focus the safe choice, not the destructive one
      cancelButton.value?.focus()
    }
  },
)

function cancel(): void {
  if (!props.busy) {
    emit('cancel')
  }
}
</script>

<template>
  <div
    v-if="open"
    class="fd-scrim"
    :data-testid="testid"
    data-state="open"
    @keydown.esc="cancel"
  >
    <div
      class="fd-modal"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
    >
      <div class="fd-panel__title">
        {{ title }}
      </div>
      <p
        v-if="message"
        class="fd-muted"
        style="margin-top: var(--fd-2)"
      >
        {{ message }}
      </p>

      <label
        v-if="requireText"
        class="fd-field"
        style="margin-top: var(--fd-4)"
      >
        <span class="fd-label">Type “{{ requireText }}” to continue</span>
        <input
          v-model="typed"
          class="fd-input"
          type="text"
          :data-testid="`${testid}-typed`"
        >
      </label>

      <div class="fd-modal__actions">
        <button
          ref="cancelButton"
          type="button"
          class="fd-btn fd-btn--ghost"
          :data-testid="`${testid}-cancel`"
          @click="cancel"
        >
          {{ cancelLabel }}
        </button>
        <button
          type="button"
          class="fd-btn"
          :class="danger ? 'fd-btn--danger' : 'fd-btn--primary'"
          :disabled="!canConfirm"
          :data-testid="`${testid}-confirm`"
          @click="emit('confirm')"
        >
          {{ confirmLabel }}
        </button>
      </div>
    </div>
  </div>
</template>
