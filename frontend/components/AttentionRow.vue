<script setup lang="ts">
/**
 * Something that wants attention, with why it wants it.
 *
 * The bar on the left is the severity; the row never shouts in colour alone,
 * because `why` says what is wrong in words.
 */
withDefaults(
  defineProps<{
    title: string
    why?: string
    severity?: 'crit' | 'warn' | 'ok'
    testid?: string
  }>(),
  { severity: 'crit', testid: 'attention-row' },
)
</script>

<template>
  <div
    class="fd-attn"
    :data-testid="testid"
    :data-state="severity"
  >
    <span
      class="fd-attn__bar"
      :class="severity"
      aria-hidden="true"
    />
    <div>
      <div class="fd-row__name">
        {{ title }}
      </div>
      <div
        v-if="why"
        class="fd-attn__why"
      >
        {{ why }}
      </div>
    </div>
    <div>
      <slot />
    </div>
  </div>
</template>
