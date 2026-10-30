<script setup lang="ts">
/**
 * The console frame, or whatever comes before it: the session is read first
 * and decides between the setup panel, the sign-in panel and the shell.
 */
import { computed, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useApi } from '~/composables/useApi'
import { useStatusPanel } from '~/composables/useStatusPanel'

const { client, ready, authenticated, setupRequired } = useApi()
const { component: statusComponent, panelProps: statusProps } = useStatusPanel()
const route = useRoute()

/** The wizard is due while the bridge has no identity or no z2m server. */
const wizardDue = ref(false)

/**
 * The name the running bridge reports. It is read from the active
 * configuration, not from the draft: the header says what the bridge is
 * called right now, and the draft may rename it later.
 */
const activeName = ref<string | null>(null)
const bridgeName = computed(() => activeName.value ?? 'Bifrost')

/**
 * Go to the wizard by changing only the hash.
 *
 * A router push resolves against the router's base, which drops the ingress
 * prefix the page was served under; the hash never does.
 */
function openWizard(): void {
  if (globalThis.location !== undefined) {
    globalThis.location.hash = '#/setup'
  }
}

/**
 * Whether the wizard still has work to do.
 *
 * A bridge with an identity and one z2m server never sees it again. In setup
 * mode there is no configuration at all, so the read fails and the answer is
 * yes.
 */
async function wizardPending(): Promise<boolean> {
  try {
    const config = (await client.activeConfig()) as {
      bridge?: { mac?: unknown }
      z2m?: Record<string, unknown>
    }
    const identity = typeof config.bridge?.mac === 'string' && config.bridge.mac !== ''
    const servers = Object.keys(config.z2m ?? {}).length > 0
    return !(identity && servers)
  }
  catch {
    return true
  }
}

watch(
  authenticated,
  async (signedIn) => {
    if (!signedIn) {
      activeName.value = null
      wizardDue.value = false
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

    wizardDue.value = await wizardPending()
    if (wizardDue.value && route.path !== '/setup') {
      openWizard()
    }
  },
  { immediate: true },
)

// the wizard finishes by applying a configuration, so the answer changes
// without the session changing
watch(
  () => route.path,
  async (path) => {
    if (!authenticated.value) {
      return
    }
    if (path === '/setup') {
      return
    }
    wizardDue.value = await wizardPending()
    if (wizardDue.value) {
      openWizard()
    }
  },
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
