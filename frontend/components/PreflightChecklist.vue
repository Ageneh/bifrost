<script setup lang="ts">
/**
 * What has to be true before a stream can run, one row at a time.
 *
 * A row is green or red and nothing else: a row that is red names the screen
 * that fixes it, so the checklist is a to-do list rather than a diagnosis
 * the reader has to translate.
 */
import type { PreflightRow } from '~/lib/entertainment'

defineProps<{
  rows: PreflightRow[]
  testid?: string
}>()
</script>

<template>
  <ul
    class="preflight"
    :data-testid="testid ?? 'preflight'"
  >
    <li
      v-for="row in rows"
      :key="row.id"
      class="preflight__row"
      :data-testid="`preflight-${row.id}`"
      :data-state="row.state"
    >
      <span
        class="preflight__dot"
        aria-hidden="true"
      />
      <span class="preflight__text">
        <span class="fd-value">{{ row.label }}</span>
        <span class="fd-meta fd-muted">{{ row.detail }}</span>
      </span>
      <NuxtLink
        v-if="row.to"
        :to="row.to"
        class="fd-btn fd-btn--ghost fd-btn--sm"
        :data-testid="`preflight-${row.id}-fix`"
      >
        Fix
      </NuxtLink>
    </li>
  </ul>
</template>

<style scoped>
.preflight {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--fd-2);
}
.preflight__row {
  display: flex;
  align-items: center;
  gap: var(--fd-3);
  padding: var(--fd-2) var(--fd-3);
  border: 1px solid var(--fd-line);
  border-radius: var(--fd-r-card);
  background: var(--fd-surface-1);
}
.preflight__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex: none;
  background: var(--fd-crit);
}
.preflight__row[data-state='ok'] .preflight__dot {
  background: var(--fd-ok);
}
.preflight__text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}
</style>
