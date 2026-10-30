<script setup lang="ts">
/**
 * The console frame, or whatever comes before it: the session is read first
 * and decides between the setup panel, the sign-in panel and the shell.
 */
import { computed, ref, watch } from 'vue'
import { useApi } from '~/composables/useApi'
import { useStatusPanel } from '~/composables/useStatusPanel'

const { client, ready, authenticated, setupRequired } = useApi()
const { component: statusComponent, panelProps: statusProps } = useStatusPanel()

/**
 * The name the running bridge reports. It is read from the active
 * configuration, not from the draft: the header says what the bridge is
 * called right now, and the draft may rename it later.
 */
const activeName = ref<string | null>(null)
const bridgeName = computed(() => activeName.value ?? 'Bifrost')

watch(
  authenticated,
  async (signedIn) => {
    if (!signedIn) {
      activeName.value = null
      return
    }
    try {
      const config = (await client.activeConfig()) as { bridge?: { name?: unknown } }
      if (typeof config.bridge?.name === 'string' && config.bridge.name !== '') {
        activeName.value = config.bridge.name
      }
    }
    catch {
      // the shell still renders; Home reports the read failure itself
    }
  },
  { immediate: true },
)
</script>

<template>
  <div class="layout-root">
    <div
      v-if="!ready"
      class="gate"
      data-testid="session-loading"
    >
      <div
        class="fd-skeleton"
        style="width: 240px; height: 40px"
      />
    </div>

    <SetupPanel v-else-if="setupRequired" />

    <SignInPanel v-else-if="!authenticated" />

    <AppShell
      v-else
      :bridge-name="bridgeName"
    >
      <slot />
      <template #status>
        <component
          :is="statusComponent"
          v-if="statusComponent"
          v-bind="statusProps"
        />
        <div class="draft-slot">
          <DraftBar />
        </div>
        <RestartBanner />
        <LanRiskNotice />
      </template>
    </AppShell>

    <DraftPill v-if="authenticated" />
  </div>
</template>

<style scoped>
.layout-root {
  min-height: 100vh;
}
.gate {
  display: grid;
  place-items: center;
  min-height: 100vh;
}
/* the status column stacks below the fold on a tablet; the pill replaces it */
@media (max-width: 1000px) {
  .draft-slot {
    display: none;
  }
}
</style>
