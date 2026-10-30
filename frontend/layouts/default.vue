<script setup lang="ts">
/**
 * The console frame, or whatever comes before it: the session is read first
 * and decides between the setup panel, the sign-in panel and the shell.
 */
import { computed } from 'vue'
import { useApi } from '~/composables/useApi'

const { ready, authenticated, setupRequired } = useApi()

const bridgeName = computed(() => 'Bifrost')
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
        <LanRiskNotice />
      </template>
    </AppShell>
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
</style>
