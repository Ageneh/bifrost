<script setup lang="ts">
/**
 * One way to get a TV streaming, as a card that can be taken.
 *
 * A recipe is not a shortcut around the editor: it creates the zone the
 * editor would have created, from the same devices and the same rules, and
 * then hands the user to it. A blocked recipe says which row of the
 * preflight stopped it.
 */
defineProps<{
  title: string
  why: string
  requirements: string[]
  /** False when the bridge cannot stream yet; the reason is in `blocked`. */
  available: boolean
  blocked?: string | null
  testid: string
}>()

defineEmits<{ start: [] }>()
</script>

<template>
  <article
    class="fd-panel fd-panel--card recipe"
    :data-testid="testid"
    :data-state="available ? 'ready' : 'blocked'"
  >
    <div class="fd-panel__body fd-stack">
      <div class="fd-between">
        <div class="fd-panel__title">
          {{ title }}
        </div>
        <span
          class="fd-pill"
          :data-testid="`${testid}-state`"
        >{{ available ? 'ready' : 'blocked' }}</span>
      </div>

      <div class="fd-panel__sub">
        {{ why }}
      </div>

      <ul class="recipe__list">
        <li
          v-for="line in requirements"
          :key="line"
          class="fd-meta fd-muted"
        >
          {{ line }}
        </li>
      </ul>

      <div
        v-if="!available && blocked"
        class="fd-alert"
        :data-testid="`${testid}-blocked`"
      >
        {{ blocked }}
      </div>

      <div class="fd-cluster">
        <button
          type="button"
          class="fd-btn fd-btn--primary fd-btn--sm"
          :disabled="!available"
          :data-testid="`${testid}-start`"
          @click="$emit('start')"
        >
          Start
        </button>
      </div>
    </div>
  </article>
</template>

<style scoped>
.recipe__list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
</style>
