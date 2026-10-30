<script setup lang="ts">
/**
 * One thing that needs attention, and the card behind it.
 *
 * The row is the cause in one sentence; opening it shows what is actually
 * wrong and the single action that fixes it. Severity is never colour alone:
 * the bar carries the colour and the word carries the meaning.
 */
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    title: string
    /** One line under the title, always visible. */
    why?: string
    /** The card: what is wrong. */
    cause?: string
    /** The card: what to do about it. */
    fix?: string
    severity?: 'crit' | 'warn' | 'ok'
    /** Label of the fix button; no button without it. */
    actionLabel?: string
    expanded?: boolean
    testid?: string
  }>(),
  {
    why: '',
    cause: '',
    fix: '',
    severity: 'crit',
    actionLabel: '',
    expanded: false,
    testid: 'attention-row',
  },
)

const emit = defineEmits<{ toggle: [], action: [] }>()

const severityLabel = computed(() => {
  switch (props.severity) {
    case 'crit':
      return 'Failed'
    case 'warn':
      return 'Warning'
    default:
      return 'Resolved'
  }
})

/** The card only exists when there is something to put in it. */
const expandable = computed(() => props.cause !== '')
const cardId = computed(() => `${props.testid}-card`)
</script>

<template>
  <div
    class="fd-attn attn"
    :data-testid="testid"
    data-attention="true"
    :data-severity="severity"
    :data-state="severity"
    :data-expanded="expanded ? 'true' : 'false'"
  >
    <span
      class="fd-attn__bar"
      :class="severity"
      aria-hidden="true"
    />
    <div class="attn__body">
      <button
        v-if="expandable"
        type="button"
        class="attn__summary"
        :aria-expanded="expanded"
        :aria-controls="cardId"
        :data-testid="`${testid}-toggle`"
        @click="emit('toggle')"
      >
        <span class="attn__text">
          <span class="fd-row__name">{{ title }}</span>
          <span
            v-if="why"
            class="fd-attn__why"
          >{{ why }}</span>
        </span>
        <span
          class="fd-status"
          :class="`fd-status--${severity}`"
          :data-testid="`${testid}-severity`"
        >{{ severityLabel }}</span>
        <svg
          class="attn__chevron"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          aria-hidden="true"
        >
          <path d="M4 6l4 4 4-4" />
        </svg>
      </button>

      <div
        v-else
        class="attn__summary"
        :data-testid="`${testid}-summary`"
      >
        <span class="attn__text">
          <span class="fd-row__name">{{ title }}</span>
          <span
            v-if="why"
            class="fd-attn__why"
          >{{ why }}</span>
        </span>
        <span
          class="fd-status"
          :class="`fd-status--${severity}`"
          :data-testid="`${testid}-severity`"
        >{{ severityLabel }}</span>
      </div>

      <div
        v-if="expanded && expandable"
        :id="cardId"
        class="attn__card"
        :data-testid="cardId"
        role="region"
        :aria-label="title"
      >
        <div>
          <div class="fd-label">
            Cause
          </div>
          <p class="attn__line">
            {{ cause }}
          </p>
        </div>
        <div v-if="fix">
          <div class="fd-label">
            Fix
          </div>
          <p class="attn__line">
            {{ fix }}
          </p>
        </div>
        <div v-if="actionLabel">
          <button
            type="button"
            class="fd-btn fd-btn--primary fd-btn--sm"
            :data-testid="`${testid}-action`"
            @click="emit('action')"
          >
            {{ actionLabel }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.attn {
  grid-template-columns: 3px minmax(0, 1fr);
}
.attn__body {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.attn__summary {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto 16px;
  align-items: start;
  gap: var(--fd-3);
  width: 100%;
  padding: 0;
  border: 0;
  background: none;
  color: inherit;
  text-align: left;
  cursor: pointer;
}
div.attn__summary {
  grid-template-columns: minmax(0, 1fr) auto;
  cursor: default;
}
.attn__text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.attn__chevron {
  width: 16px;
  height: 16px;
  color: var(--fd-text-3);
  transition: transform var(--fd-fast);
  margin-top: 2px;
}
.attn[data-expanded='true'] .attn__chevron {
  transform: rotate(180deg);
}
.attn__card {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  padding: var(--fd-3);
  border: 1px solid var(--fd-line);
  border-radius: var(--fd-r-inset);
  background: var(--fd-surface-2);
}
.attn__line {
  color: var(--fd-text-2);
  font-size: var(--fd-size-sm);
  margin-top: 3px;
}
@media (prefers-reduced-motion: reduce) {
  .attn__chevron {
    transition: none;
  }
}
</style>
