<script setup lang="ts">
/**
 * The draft, grouped by the section it touches.
 *
 * Grouping is by configuration section, not by blast radius: the server only
 * classifies a change when it applies one, and a label that guessed "hot"
 * would be a lie. Wave 3 adds the classification once apply reports it.
 */
import { computed } from 'vue'
import type { ConfigChange } from '~/lib/generated/ConfigChange'
import ChangeRow from './ChangeRow.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    changes: readonly ConfigChange[]
    revision: number
    testid?: string
  }>(),
  { testid: 'diff-drawer' },
)

const emit = defineEmits<{ close: [] }>()

const SECTION_LABELS: Record<string, string> = {
  bridge: 'Bridge and network',
  z2m: 'zigbee2mqtt servers',
  rooms: 'Rooms',
  bifrost: 'Bifrost',
}

const groups = computed(() => {
  const bySection = new Map<string, ConfigChange[]>()
  for (const change of props.changes) {
    const section = change.path.split('.')[0] ?? change.path
    const bucket = bySection.get(section) ?? []
    bucket.push(change)
    bySection.set(section, bucket)
  }
  return [...bySection.entries()].map(([section, changes]) => ({
    section,
    label: SECTION_LABELS[section] ?? section,
    changes,
  }))
})
</script>

<template>
  <div
    v-if="open"
    class="fd-scrim"
    :data-testid="testid"
    data-state="open"
    @click.self="emit('close')"
  >
    <div
      class="drawer fd-panel"
      role="dialog"
      aria-modal="true"
      aria-label="Draft changes"
    >
      <div class="fd-panel__head">
        <div>
          <div class="fd-panel__title">
            Draft · {{ changes.length }} change{{ changes.length === 1 ? '' : 's' }}
          </div>
          <div class="fd-panel__sub">
            Revision {{ revision }}
          </div>
        </div>
        <button
          type="button"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          :data-testid="`${testid}-close`"
          @click="emit('close')"
        >
          Close
        </button>
      </div>

      <div
        v-if="groups.length === 0"
        class="fd-empty"
        data-testid="diff-empty"
      >
        <span class="fd-muted">The draft matches the active configuration.</span>
      </div>

      <div
        v-for="group in groups"
        :key="group.section"
        class="fd-stack"
        :data-testid="`diff-group-${group.section}`"
      >
        <div class="fd-label">
          {{ group.label }}
        </div>
        <div class="fd-list">
          <div
            v-for="change in group.changes"
            :key="change.path"
            class="fd-row"
            style="cursor: default"
          >
            <ChangeRow :change="change" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.drawer {
  width: min(560px, 100%);
  max-height: min(80vh, 720px);
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: var(--fd-4);
}
</style>
