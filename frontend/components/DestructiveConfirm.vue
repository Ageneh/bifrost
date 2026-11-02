<script setup lang="ts">
/**
 * A confirmation for an action that cannot be undone.
 *
 * Three things are always on it: what the action costs (one line per
 * consequence), a backup the user may take first, and a typed word that
 * proves the user meant it. The backup is offered, never taken: nothing
 * downloads on its own.
 */
import ConfirmDialog from '~/components/ConfirmDialog.vue'
import ImpactList from '~/components/ImpactList.vue'

withDefaults(
  defineProps<{
    open: boolean
    title: string
    message?: string
    confirmLabel?: string
    requireText?: string
    /** One line per consequence, shown before the typed confirmation. */
    impacts?: readonly string[]
    /** The label of the backup button; empty hides it. */
    backupLabel?: string
    backupBusy?: boolean
    /** Why the backup cannot be taken, when it cannot. */
    backupNote?: string
    busy?: boolean
    testid?: string
  }>(),
  {
    message: '',
    confirmLabel: 'Confirm',
    requireText: '',
    impacts: () => [],
    backupLabel: '',
    backupBusy: false,
    backupNote: '',
    busy: false,
    testid: 'destructive-confirm',
  },
)

const emit = defineEmits<{ confirm: [], cancel: [], backup: [] }>()
</script>

<template>
  <ConfirmDialog
    :open="open"
    :title="title"
    :message="message"
    :confirm-label="confirmLabel"
    :require-text="requireText"
    :busy="busy"
    danger
    :testid="testid"
    @confirm="emit('confirm')"
    @cancel="emit('cancel')"
  >
    <!-- an action that needs a value, e.g. the new MAC address -->
    <slot />

    <div
      v-if="impacts.length > 0"
      style="margin-top: var(--fd-3)"
      :data-testid="`${testid}-impacts`"
    >
      <ImpactList
        :lines="impacts"
        :testid="`${testid}-impact-list`"
      />
    </div>

    <div
      v-if="backupLabel"
      style="margin-top: var(--fd-3)"
      :data-testid="`${testid}-backup-row`"
    >
      <button
        type="button"
        class="fd-btn fd-btn--sm"
        :disabled="backupBusy"
        :data-testid="`${testid}-backup`"
        @click="emit('backup')"
      >
        {{ backupBusy ? 'Preparing…' : backupLabel }}
      </button>
      <p
        v-if="backupNote"
        class="fd-muted"
        style="margin-top: var(--fd-1)"
        :data-testid="`${testid}-backup-note`"
      >
        {{ backupNote }}
      </p>
    </div>
  </ConfirmDialog>
</template>
