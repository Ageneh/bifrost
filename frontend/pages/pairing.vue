<script setup lang="ts">
/**
 * Pairing: the link button, the window it opens, and who has paired.
 *
 * The person pressing this button is usually holding a phone, not looking at
 * the status column, so an open window also shows as a banner pinned to the
 * top of the page at every width. The mode the bridge runs in — compat or
 * required — is always on screen: it decides whether a window matters at all.
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import LinkButtonCard from '~/components/LinkButtonCard.vue'
import PairingInstructions from '~/components/PairingInstructions.vue'
import UserTable from '~/components/UserTable.vue'
import { useApi } from '~/composables/useApi'
import { useLiveSocket } from '~/composables/useLiveSocket'
import { useToast } from '~/composables/useToast'
import type { LinkButtonStatus } from '~/lib/generated/LinkButtonStatus'
import type { PairingHistory } from '~/lib/generated/PairingHistory'

/** The window the button asks for. */
const WINDOW_SECONDS = 30

const { client } = useApi()
const toast = useToast()
const socket = useLiveSocket()

const history = ref<PairingHistory | null>(null)
const pressed = ref(false)
const remaining = ref(0)
const loading = ref(true)
const failure = ref<string | null>(null)
const pressing = ref(false)

/** Tick the countdown locally; the server stays the authority on every read. */
let ticker: ReturnType<typeof setInterval> | null = null

const mode = computed(() => history.value?.mode ?? 'compat')
const records = computed(() => history.value?.records ?? [])

const modeLabel = computed(() => (mode.value === 'required' ? 'required' : 'compat'))
const modeExplanation = computed(() =>
  mode.value === 'required'
    ? 'A client pairs only while a window is open. A client that asks outside one is refused.'
    : 'Every client may pair at any time. The window is informational only.',
)

function stopTicker(): void {
  if (ticker !== null) {
    clearInterval(ticker)
    ticker = null
  }
}

function applyStatus(status: LinkButtonStatus): void {
  pressed.value = status.pressed
  remaining.value = status.remaining_seconds
  stopTicker()
  if (!status.pressed) {
    return
  }
  ticker = setInterval(() => {
    remaining.value -= 1
    if (remaining.value <= 0) {
      pressed.value = false
      remaining.value = 0
      stopTicker()
    }
  }, 1000)
}

async function refresh(): Promise<void> {
  try {
    const [pairing, status] = await Promise.all([client.pairing(), client.linkButton()])
    history.value = pairing
    applyStatus(status)
    failure.value = null
  }
  catch (cause) {
    failure.value = cause instanceof Error ? cause.message : 'Could not read the pairing state.'
  }
  finally {
    loading.value = false
  }
}

async function press(): Promise<void> {
  pressing.value = true
  try {
    const status = await client.pressLinkButton(WINDOW_SECONDS)
    applyStatus(status)
    toast.ok('Pairing window open. Pair from the client now.')
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not open the pairing window.')
  }
  finally {
    pressing.value = false
  }
}

let unsubscribe: (() => void) | null = null

onMounted(async () => {
  await refresh()
  socket.connect()
  unsubscribe = socket.onUpdate((update) => {
    if ('LinkButton' in update) {
      applyStatus(update.LinkButton)
      // a pairing that just landed changes the history
      void refresh()
    }
  })
})

onBeforeUnmount(() => {
  stopTicker()
  unsubscribe?.()
})
</script>

<template>
  <div
    class="fd-stack"
    data-testid="pairing-screen"
  >
    <!-- the person pressing the button is holding a phone: the banner is
         pinned, not a column that stacks below the fold -->
    <div
      v-if="pressed"
      class="fd-notice"
      style="position: sticky; top: 0; z-index: 20"
      role="status"
      aria-live="polite"
      data-testid="pairing-banner"
      data-state="open"
      :data-remaining="remaining"
    >
      <strong>Pairing window open — {{ remaining }}s left.</strong>
      Pair from the client now. A second client in the same window is refused.
    </div>

    <header class="fd-between">
      <div>
        <div class="fd-label">
          Pairing
        </div>
        <h1 class="fd-panel__title">
          Link button and clients
        </h1>
      </div>
      <div class="fd-cluster">
        <span class="fd-label">Mode</span>
        <span
          class="fd-badge"
          :data-state="mode === 'required' ? 'warn' : 'ok'"
          data-testid="pairing-mode"
        >{{ modeLabel }}</span>
      </div>
    </header>

    <p
      class="fd-muted"
      data-testid="pairing-mode-explanation"
    >
      {{ modeExplanation }}
    </p>

    <AsyncState
      :loading="loading && history === null"
      :error="history === null ? failure : null"
      :skeleton-rows="4"
      testid="pairing-state"
      @retry="refresh"
    >
      <LinkButtonCard
        :pressed="pressed"
        :remaining="remaining"
        :window-seconds="WINDOW_SECONDS"
        :busy="pressing"
        @press="press"
      />

      <PairingInstructions />

      <UserTable
        :records="records"
        :loading="loading"
      />
    </AsyncState>
  </div>
</template>
