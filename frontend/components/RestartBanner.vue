<script setup lang="ts">
/**
 * What the last apply left behind: settings that only take effect after a
 * restart. The banner stays until the restart happens, because the draft is
 * already applied and only the running process is stale.
 */
import { computed } from 'vue'
import { useDraft } from '~/composables/useDraft'
import { useToast } from '~/composables/useToast'
import ImpactList from './ImpactList.vue'

const draft = useDraft()
const toast = useToast()

const reasons = computed(() => draft.restartRequired.value)
const visible = computed(() => reasons.value.length > 0)

async function restart(): Promise<void> {
  const asked = await draft.restart()
  if (asked) {
    toast.ok('Restart requested. The console reconnects on its own.')
  }
  else if (draft.error.value) {
    toast.fail(draft.error.value)
  }
}
</script>

<template>
  <div
    v-if="visible"
    class="fd-stack"
    data-testid="restart-banner"
    data-state="restart"
  >
    <div class="fd-panel fd-panel--card">
      <div class="fd-between">
        <div>
          <div class="fd-panel__title">
            Restart needed
          </div>
          <div class="fd-panel__sub">
            {{ reasons.length }} setting{{ reasons.length === 1 ? '' : 's' }} take effect only after
            Bifrost restarts.
          </div>
        </div>
        <button
          type="button"
          class="fd-btn fd-btn--sm"
          data-testid="restart-now"
          @click="restart"
        >
          Restart now
        </button>
      </div>
      <div style="margin-top: var(--fd-3)">
        <ImpactList
          :reasons="reasons"
          testid="restart-impact"
        />
      </div>
    </div>
  </div>
</template>
