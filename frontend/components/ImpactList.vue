<script setup lang="ts">
/**
 * What applying a change will cost, before it is applied.
 *
 * Each line names a consequence ("Restarting the HTTP listener drops 1
 * stream"), never a setting: the user is deciding whether to accept the
 * consequence, not re-reading the diff.
 */
import type { Reason } from '~/lib/generated/Reason'

withDefaults(
  defineProps<{
    reasons?: readonly Reason[]
    /** Plain lines for consequences that are not settings. */
    lines?: readonly string[]
    testid?: string
  }>(),
  { reasons: () => [], lines: () => [], testid: 'impact-list' },
)
</script>

<template>
  <div
    class="fd-impact"
    :data-testid="testid"
    data-state="restart"
  >
    <ul style="margin: 0; padding-left: var(--fd-4)">
      <li
        v-for="line in lines"
        :key="line"
      >
        {{ line }}
      </li>
      <li
        v-for="reason in reasons"
        :key="reason.path"
      >
        <span class="mono">{{ reason.path }}</span> — {{ reason.message }}
      </li>
    </ul>
  </div>
</template>
