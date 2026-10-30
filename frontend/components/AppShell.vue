<script setup lang="ts">
/**
 * The console frame: rail, primary view, status column.
 *
 * Narrow widths rearrange the same three regions rather than hiding one:
 * below 1000 px the status column drops under the primary view, and below
 * 600 px the rail becomes a bottom tab bar, because a thumb reaches the
 * bottom of a phone and not the top.
 */
import StatusPill from './StatusPill.vue'
import SideNav from './SideNav.vue'
import type { StatusState } from './StatusPill.vue'

withDefaults(
  defineProps<{
    bridgeName?: string
    statusLabel?: string
    statusState?: StatusState
    testid?: string
  }>(),
  { bridgeName: 'Bifrost', statusLabel: 'live', statusState: 'live', testid: 'app-shell' },
)
</script>

<template>
  <div
    class="shell"
    :data-testid="testid"
  >
    <aside
      class="shell__rail"
      data-testid="shell-rail"
    >
      <SideNav />
    </aside>

    <main
      class="shell__main"
      data-testid="shell-primary"
    >
      <header
        class="fd-between"
        style="margin-bottom: var(--fd-4)"
      >
        <div>
          <div class="fd-label">
            Bifrost Control
          </div>
          <div
            class="fd-panel__title"
            data-testid="shell-bridge-name"
          >
            {{ bridgeName }}
          </div>
        </div>
        <StatusPill
          :label="statusLabel"
          :state="statusState"
          testid="shell-status"
        />
      </header>
      <slot />
    </main>

    <aside
      class="shell__status"
      data-testid="shell-status-column"
    >
      <slot name="status" />
    </aside>
  </div>
</template>

<style scoped>
.shell {
  display: grid;
  grid-template-columns: 56px minmax(0, 1fr) 320px;
  grid-template-rows: 1fr;
  gap: var(--fd-3);
  min-height: 100%;
  padding: var(--fd-3);
  background: var(--fd-bg);
}
.shell__rail {
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--fd-surface-1);
  border: 1px solid var(--fd-line);
  border-radius: var(--fd-r-panel);
  padding: var(--fd-2);
}
.shell__rail :deep(.fd-nav__label),
.shell__rail :deep(.fd-meta) {
  display: none;
}
.shell__main,
.shell__status {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}

/* tablet: the status column joins the stack */
@media (max-width: 1000px) {
  .shell {
    grid-template-columns: 56px minmax(0, 1fr);
    grid-template-rows: auto auto;
  }
  .shell__status {
    grid-column: 2;
  }
}

/* phone: the rail is a bottom tab bar */
@media (max-width: 600px) {
  .shell {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto auto;
    padding: var(--fd-2);
    padding-bottom: calc(64px + env(safe-area-inset-bottom, 0px));
  }
  .shell__rail {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 40;
    flex-direction: row;
    justify-content: space-around;
    border-radius: 0;
    border-left: 0;
    border-right: 0;
    border-bottom: 0;
    padding: var(--fd-2) var(--fd-1) calc(var(--fd-2) + env(safe-area-inset-bottom, 0px));
    background: var(--fd-surface-1);
  }
  .shell__rail :deep(.fd-nav) {
    flex-direction: row;
    flex: 1;
    justify-content: space-around;
  }
  .shell__rail :deep(.fd-nav__item) {
    flex-direction: column;
    gap: 2px;
    padding: var(--fd-1);
    font-size: var(--fd-size-micro);
  }
  .shell__rail :deep(.fd-nav__label) {
    display: block;
  }
  .shell__rail :deep(.fd-meta) {
    display: none;
  }
  .shell__status {
    grid-column: 1;
  }
}
</style>
