<script setup lang="ts">
/**
 * The System tabs, in the order the concept fixes them.
 *
 * A tab whose screen is not built yet stays visible and disabled, with the
 * reason, rather than disappearing and leaving the set looking smaller than
 * it is. The ticket that builds a screen flips its tab to a link.
 */
export interface SystemTab {
  key: string
  label: string
  path?: string
  pending?: boolean
}

const tabs: SystemTab[] = [
  { key: 'bridge', label: 'Bridge & network', path: '/system/bridge' },
  { key: 'z2m', label: 'Zigbee2MQTT', pending: true },
  { key: 'services', label: 'Services', pending: true },
  { key: 'logs', label: 'Logs', pending: true },
  { key: 'maintenance', label: 'Maintenance', pending: true },
]

withDefaults(
  defineProps<{
    current: string
    testid?: string
  }>(),
  { testid: 'system-tabs' },
)
</script>

<template>
  <nav
    class="tabs"
    :data-testid="testid"
    aria-label="System"
  >
    <template
      v-for="tab in tabs"
      :key="tab.key"
    >
      <NuxtLink
        v-if="tab.path"
        :to="tab.path"
        class="tab"
        :data-state="tab.key === current ? 'active' : 'idle'"
        :data-testid="`system-tab-${tab.key}`"
        :aria-current="tab.key === current ? 'page' : undefined"
      >
        {{ tab.label }}
      </NuxtLink>
      <span
        v-else
        class="tab"
        data-state="pending"
        :data-testid="`system-tab-${tab.key}`"
        aria-disabled="true"
      >
        {{ tab.label }}
        <span class="fd-meta">soon</span>
      </span>
    </template>
  </nav>
</template>

<style scoped>
.tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--fd-1) var(--fd-2);
  border-bottom: 1px solid var(--fd-line);
  padding-bottom: var(--fd-2);
}
.tab {
  display: inline-flex;
  align-items: center;
  gap: var(--fd-1);
  padding: var(--fd-1) var(--fd-3);
  border-radius: var(--fd-r-control);
  color: var(--fd-text-2);
  font-size: var(--fd-size-sm);
  font-weight: 550;
}
.tab:hover {
  background: var(--fd-surface-2);
  color: var(--fd-text);
}
.tab[data-state='active'] {
  background: var(--fd-accent-soft);
  color: var(--fd-accent-text);
}
.tab[data-state='pending'] {
  color: var(--fd-text-3);
  cursor: not-allowed;
}
.tab[data-state='pending']:hover {
  background: none;
  color: var(--fd-text-3);
}
@media (max-width: 600px) {
  .tabs {
    overflow-x: auto;
    flex-wrap: nowrap;
    scrollbar-width: none;
  }
  .tab {
    white-space: nowrap;
  }
}
</style>
