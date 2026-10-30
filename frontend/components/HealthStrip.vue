<script setup lang="ts">
/**
 * The health strip: one line of facts about the running bridge.
 *
 * It is deliberately not a row of KPI tiles. Each segment is a label and a
 * state in words, in a fixed order, so the same fact sits in the same place
 * every time the page loads. The dot carries the colour; the value carries
 * the meaning.
 */
import type { HealthItem } from '~/lib/doctor'

withDefaults(
  defineProps<{
    items: readonly HealthItem[]
    testid?: string
  }>(),
  { testid: 'health-strip' },
)
</script>

<template>
  <div
    class="fd-panel strip"
    :data-testid="testid"
  >
    <div
      v-for="item in items"
      :key="item.id"
      class="strip__item"
      :data-state="item.state"
      :data-testid="`${testid}-${item.id}`"
    >
      <span
        class="strip__dot"
        aria-hidden="true"
      />
      <span class="fd-label">{{ item.label }}</span>
      <span class="strip__value fd-num">{{ item.value }}</span>
    </div>
  </div>
</template>

<style scoped>
.strip {
  display: flex;
  flex-wrap: wrap;
  gap: var(--fd-3) var(--fd-6);
  padding: var(--fd-3) var(--fd-4);
}
.strip__item {
  display: flex;
  align-items: baseline;
  gap: var(--fd-2);
  min-width: 0;
}
.strip__dot {
  width: 7px;
  height: 7px;
  border-radius: var(--fd-r-pill);
  background: var(--fd-text-3);
  align-self: center;
  flex: none;
}
.strip__item[data-state='ok'] .strip__dot {
  background: var(--fd-ok);
}
.strip__item[data-state='warn'] .strip__dot {
  background: var(--fd-warn);
}
.strip__item[data-state='crit'] .strip__dot {
  background: var(--fd-crit);
}
.strip__value {
  font-size: var(--fd-size-sm);
  color: var(--fd-text);
}
.strip__item[data-state='crit'] .strip__value {
  color: var(--fd-crit-text);
}
@media (max-width: 600px) {
  .strip {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--fd-2) var(--fd-3);
  }
}
</style>
