<script setup lang="ts">
/**
 * The draft on a narrow screen: one pill, one tap to its actions.
 *
 * Below 1000 px the status column stacks under the primary view, where the
 * DraftBar would be off-screen while the user edits. The pill floats above
 * the content instead, and expands to the same actions.
 */
import { computed, ref } from 'vue'
import { useDraft } from '~/composables/useDraft'
import { useToast } from '~/composables/useToast'

const draft = useDraft()
const toast = useToast()

const expanded = ref(false)

const count = computed(() => draft.changeCount.value)
const label = computed(() => {
  if (draft.status.value === 'applying') {
    return 'Applying'
  }
  if (draft.status.value === 'conflict') {
    return 'Conflict'
  }
  if (count.value === 0) {
    return 'No draft'
  }
  return `${count.value} change${count.value === 1 ? '' : 's'}`
})

const canApply = computed(
  () => draft.hasDraft.value && draft.blocking.value.length === 0 && draft.status.value !== 'applying',
)

async function apply(): Promise<void> {
  expanded.value = false
  const applied = await draft.apply()
  if (applied) {
    toast.ok('Draft applied.')
  }
  else if (draft.error.value) {
    toast.fail(draft.error.value)
  }
}
</script>

<template>
  <div
    class="pillwrap"
    data-testid="draft-pill"
    :data-state="draft.status.value"
  >
    <div
      v-if="expanded"
      class="pillwrap__actions"
    >
      <button
        type="button"
        class="fd-btn fd-btn--sm"
        data-testid="draft-pill-validate"
        @click="draft.validate()"
      >
        Validate
      </button>
      <button
        type="button"
        class="fd-btn fd-btn--primary fd-btn--sm"
        data-testid="draft-pill-apply"
        :disabled="!canApply"
        @click="apply"
      >
        Apply
      </button>
    </div>

    <button
      type="button"
      class="fd-btn fd-btn--primary fd-btn--pill pillwrap__pill"
      :aria-expanded="expanded"
      data-testid="draft-pill-toggle"
      @click="expanded = !expanded"
    >
      <span class="fd-num">{{ label }}</span>
    </button>
  </div>
</template>

<style scoped>
.pillwrap {
  position: fixed;
  right: var(--fd-4);
  bottom: calc(76px + env(safe-area-inset-bottom, 0px));
  display: none;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--fd-2);
  z-index: 45;
}
.pillwrap__actions {
  display: flex;
  gap: var(--fd-2);
}
.pillwrap__pill {
  height: 40px;
  padding: 0 var(--fd-5);
  box-shadow: var(--fd-shadow-pop);
}

/* the bar lives in the status column, which is below the fold under 1000 px */
@media (max-width: 1000px) {
  .pillwrap {
    display: flex;
  }
}
</style>
