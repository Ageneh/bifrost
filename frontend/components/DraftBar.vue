<script setup lang="ts">
/**
 * The draft, at the top of the status column.
 *
 * It says how many changes are uncommitted and offers exactly four things:
 * look at them, check them, apply them, or throw them away. Discard asks
 * first, because it is the only one that destroys work.
 */
import { computed, ref } from 'vue'
import ConfirmDialog from './ConfirmDialog.vue'
import DiffDrawer from './DiffDrawer.vue'
import { useDraft } from '~/composables/useDraft'
import { useToast } from '~/composables/useToast'

const draft = useDraft()
const toast = useToast()

const drawerOpen = ref(false)
const confirmingDiscard = ref(false)

const statusLabel = computed(() => {
  switch (draft.status.value) {
    case 'loading':
      return 'loading'
    case 'clean':
      return 'no draft'
    case 'dirty':
      return 'uncommitted'
    case 'conflict':
      return 'conflict'
    case 'applying':
      return 'applying'
    case 'applied':
      return 'applied'
    case 'failed':
      return 'failed'
    default:
      return draft.status.value
  }
})

const canApply = computed(
  () => draft.hasDraft.value && draft.blocking.value.length === 0 && draft.status.value !== 'applying',
)

async function validate(): Promise<void> {
  const findings = await draft.validate()
  const errors = findings.filter(finding => finding.severity === 'error').length
  if (errors > 0) {
    toast.fail(`${errors} problem${errors === 1 ? '' : 's'} in the draft.`)
  }
  else {
    toast.ok('Draft is valid.')
  }
}

async function apply(): Promise<void> {
  const applied = await draft.apply()
  if (applied) {
    toast.ok('Draft applied.')
  }
  else if (draft.error.value) {
    toast.fail(draft.error.value)
  }
}

async function discard(): Promise<void> {
  confirmingDiscard.value = false
  const discarded = await draft.discard()
  if (discarded) {
    toast.ok('Draft discarded.')
  }
  else if (draft.error.value) {
    toast.fail(draft.error.value)
  }
}
</script>

<template>
  <div class="fd-stack">
    <div
      class="fd-draft"
      data-testid="draft-bar"
      :data-state="draft.status.value"
    >
      <div class="fd-cluster">
        <span
          class="fd-draft__count fd-num"
          data-testid="draft-count"
        >{{ draft.changeCount.value }}</span>
        <div class="fd-grow">
          <div class="fd-value">
            {{ statusLabel }}
          </div>
          <div class="fd-meta">
            revision {{ draft.revision.value }}
            <template v-if="draft.blocking.value.length > 0">
              · {{ draft.blocking.value.length }} blocking
            </template>

            <style scoped>
              .draft-actions {
              justify-content: flex-end;
              flex-wrap: wrap;
              }
            </style>
          </div>
        </div>
      </div>
      <div class="fd-cluster draft-actions">
        <button
          type="button"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          data-testid="draft-discard"
          :disabled="!draft.hasDraft.value || draft.status.value === 'applying'"
          @click="confirmingDiscard = true"
        >
          Discard
        </button>
        <button
          type="button"
          class="fd-btn fd-btn--sm"
          data-testid="draft-validate"
          :disabled="draft.status.value === 'applying'"
          @click="validate"
        >
          Validate
        </button>
        <button
          type="button"
          class="fd-btn fd-btn--sm"
          data-testid="draft-diff"
          :disabled="draft.changeCount.value === 0"
          @click="drawerOpen = true"
        >
          Diff
        </button>
        <button
          type="button"
          class="fd-btn fd-btn--primary fd-btn--sm"
          :class="{ 'is-busy': draft.status.value === 'applying' }"
          data-testid="draft-apply"
          :disabled="!canApply"
          @click="apply"
        >
          Apply
        </button>
      </div>
    </div>

    <DiffDrawer
      :open="drawerOpen"
      :changes="draft.changes.value"
      :revision="draft.revision.value"
      @close="drawerOpen = false"
    />

    <ConfirmDialog
      :open="confirmingDiscard"
      title="Discard the draft?"
      message="Every uncommitted change is dropped. The active configuration is untouched."
      confirm-label="Discard"
      :danger="true"
      testid="discard-dialog"
      @confirm="discard"
      @cancel="confirmingDiscard = false"
    />
  </div>
</template>
