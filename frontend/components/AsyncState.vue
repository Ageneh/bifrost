<script setup lang="ts">
/**
 * The three states every panel can be in before it shows data: loading,
 * failed, empty. Errors are announced (`role="alert"`), and the retry is a
 * real button so it is reachable by keyboard.
 */
withDefaults(
  defineProps<{
    loading?: boolean
    error?: string | null
    empty?: boolean
    emptyText?: string
    /** Rows of skeleton to show while loading. */
    skeletonRows?: number
    testid?: string
  }>(),
  { loading: false, error: null, empty: false, emptyText: 'Nothing here yet.', skeletonRows: 3, testid: 'async-state' },
)

defineEmits<{ retry: [] }>()
</script>

<template>
  <div
    :data-testid="testid"
    :data-state="loading ? 'loading' : error ? 'error' : empty ? 'empty' : 'ready'"
  >
    <div
      v-if="loading"
      class="fd-stack"
      aria-busy="true"
      data-testid="async-loading"
    >
      <div
        v-for="row in skeletonRows"
        :key="row"
        class="fd-skeleton"
        style="height: 34px"
      />
    </div>

    <div
      v-else-if="error"
      class="fd-stack"
      role="alert"
      data-testid="async-error"
    >
      <div class="fd-alert">
        {{ error }}
      </div>
      <div>
        <button
          type="button"
          class="fd-btn fd-btn--sm"
          data-testid="async-retry"
          @click="$emit('retry')"
        >
          Try again
        </button>
      </div>
    </div>

    <div
      v-else-if="empty"
      class="fd-empty"
      data-testid="async-empty"
    >
      <span class="fd-muted">{{ emptyText }}</span>
    </div>

    <slot v-else />
  </div>
</template>
