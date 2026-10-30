<script setup lang="ts">
/**
 * Home: the bridge as a doctor, not a dashboard.
 *
 * Fixed order, and nothing decorative: the attention list first (critical
 * rows, then warnings, then one resolved row), the health strip, at most
 * three quick actions, and the draft last. Every row names its cause and
 * opens the fix. A healthy bridge is one row that says so.
 *
 * Every fact comes from the API or the live socket. Sources that this wave
 * does not have yet — link button, pairing, streams — contribute no row
 * rather than a placeholder.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import AttentionRow from '~/components/AttentionRow.vue'
import DiffDrawer from '~/components/DiffDrawer.vue'
import HealthStrip from '~/components/HealthStrip.vue'
import { useApi } from '~/composables/useApi'
import { useDraft } from '~/composables/useDraft'
import { useInventory } from '~/composables/useInventory'
import { useLiveSocket } from '~/composables/useLiveSocket'
import { useToast } from '~/composables/useToast'
import type { AppConfig } from '~/lib/generated/AppConfig'
import type { Service } from '~/lib/generated/Service'
import type { AttentionItem, DoctorEvent } from '~/lib/doctor'
import { deriveAttention, deriveHealth } from '~/lib/doctor'

const { client } = useApi()
const socket = useLiveSocket()
const draft = useDraft()
const inventory = useInventory()
const toast = useToast()

const config = ref<AppConfig | null>(null)
const services = ref<Service[]>([])
const loading = ref(true)
const failure = ref<string | null>(null)

/** A service that came back while this page was open. */
const recovered = ref<DoctorEvent | null>(null)
/** The last apply this session saw succeed. */
const applied = ref<DoctorEvent | null>(null)

const openRow = ref<string | null>(null)
const drawerOpen = ref(false)

/** The state each service had at the previous observation. */
const previousStates = new Map<string, string>()

function clock(): string {
  return new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

function noteRecovery(previous: Map<string, string>, next: Service[]): void {
  for (const service of next) {
    const before = previous.get(service.id)
    const now = String(service.state)
    if (before !== undefined && before !== 'Running' && now === 'Running') {
      recovered.value = { name: service.name, at: clock() }
    }
  }
}

const doctorInput = computed(() => ({
  config: config.value,
  services: services.value,
  revision: draft.revision.value,
  changeCount: draft.changeCount.value,
  draftStatus: draft.status.value,
  restartRequired: draft.restartRequired.value,
  recovered: recovered.value,
  applied: applied.value,
  readError: failure.value,
  draftError: draft.error.value,
}))

const attention = computed(() => deriveAttention(doctorInput.value))
const health = computed(() => deriveHealth(doctorInput.value))

const openCount = computed(
  () => attention.value.filter(item => item.severity !== 'ok').length,
)

const heading = computed(() => {
  if (loading.value && config.value === null) {
    return 'Reading the bridge'
  }
  if (openCount.value === 0) {
    return 'Nothing needs attention'
  }
  return `${openCount.value} thing${openCount.value === 1 ? '' : 's'} need attention`
})

const canApply = computed(
  () => draft.hasDraft.value && draft.blocking.value.length === 0 && draft.status.value !== 'applying',
)

interface QuickAction {
  id: string
  label: string
  primary?: boolean
  run: () => void | Promise<void>
}

/**
 * At most three actions, and only ones that do something right now: a
 * button that cannot change anything is a decoration.
 */
const quickActions = computed<QuickAction[]>(() => {
  const actions: QuickAction[] = []
  if (draft.restartRequired.value.length > 0) {
    actions.push({ id: 'restart', label: 'Restart now', primary: true, run: restartBridge })
  }
  if (canApply.value) {
    actions.push({ id: 'apply', label: 'Apply draft', primary: actions.length === 0, run: applyDraft })
  }
  if (draft.changeCount.value > 0) {
    actions.push({ id: 'review', label: 'Review draft', run: openDraft })
  }
  actions.push({ id: 'bridge', label: 'Bridge settings', run: openBridge })
  // pairing belongs here as well as on the Devices head, but it is never
  // more urgent than a draft or a restart, so it queues behind them
  if (runningServer.value !== undefined) {
    actions.push({ id: 'pair', label: 'Pair a device', run: pairDevice })
  }
  return actions.slice(0, 3)
})

async function load(): Promise<void> {
  loading.value = true
  try {
    const [active, list] = await Promise.all([client.activeConfig(), client.services()])
    // the active configuration is redacted but otherwise the same document
    config.value = active as unknown as AppConfig
    const next = Object.values(list.services)
    noteRecovery(previousStates, next)
    previousStates.clear()
    for (const service of next) {
      previousStates.set(service.id, String(service.state))
    }
    services.value = next
    failure.value = null
  }
  catch (cause) {
    failure.value = cause instanceof Error ? cause.message : 'Could not read the bridge state.'
  }
  finally {
    loading.value = false
  }
}

let stopUpdates: (() => void) | null = null

function applyUpdate(update: Service): void {
  const index = services.value.findIndex(service => service.id === update.id)
  if (index >= 0) {
    services.value[index] = update
  }
  else {
    services.value = [...services.value, update]
  }
  noteRecovery(previousStates, [update])
  previousStates.set(update.id, String(update.state))
}

onMounted(async () => {
  stopUpdates = socket.onUpdate((update) => {
    if ('AppConfig' in update) {
      config.value = update.AppConfig
    }
    else if ('ServiceUpdate' in update) {
      applyUpdate(update.ServiceUpdate)
    }
  })
  await load()
  await draft.load()
  await inventory.refresh()
})

onBeforeUnmount(() => {
  stopUpdates?.()
  stopUpdates = null
})

// an apply that succeeded is the clearest thing the doctor can report
watch(
  () => draft.status.value,
  (status) => {
    if (status === 'applied') {
      applied.value = { name: 'Draft', at: clock() }
    }
  },
)

function openDraft(): void {
  drawerOpen.value = true
}

function toggleRow(id: string): void {
  openRow.value = openRow.value === id ? null : id
}

async function restartService(serviceId: string): Promise<void> {
  try {
    await client.setServiceState(serviceId, 'Running')
    toast.ok('Restart requested.')
    await load()
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'The service did not restart.')
  }
}

async function applyDraft(): Promise<void> {
  const done = await draft.apply()
  if (done) {
    toast.ok('Draft applied.')
  }
  else if (draft.error.value) {
    toast.fail(draft.error.value)
  }
}

async function restartBridge(): Promise<void> {
  const done = await draft.restart()
  if (done) {
    toast.ok('Restart requested. The console reconnects on its own.')
  }
  else if (draft.error.value) {
    toast.fail(draft.error.value)
  }
}

function openBridge(): void {
  void navigateTo('/system/bridge')
}

/** The z2m server a join window would be opened on. */
const runningServer = computed(
  () => inventory.servers.value.find(server => server.online)?.name,
)

/** Open a join window on the running server, so a new device can join. */
async function pairDevice(): Promise<void> {
  const target = runningServer.value
  if (target === undefined) {
    toast.fail('No z2m server is running, so nothing can be paired.')
    return
  }
  try {
    const window = await inventory.permitJoin(target)
    toast.ok(`Join window open on ${target} for ${window.time} seconds.`)
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'The join window did not open.')
  }
}

function runAction(item: AttentionItem): void {
  const action = item.action
  if (action === undefined) {
    return
  }
  switch (action.kind) {
    case 'restart-service':
      if (action.serviceId !== undefined) {
        void restartService(action.serviceId)
      }
      break
    case 'review-draft':
      drawerOpen.value = true
      break
    case 'apply-draft':
      void applyDraft()
      break
    case 'restart-bridge':
      void restartBridge()
      break
    case 'open-bridge':
      openBridge()
      break
  }
}
</script>

<template>
  <div
    class="fd-stack"
    data-testid="home-view"
  >
    <AsyncState
      :loading="loading && config === null"
      :error="config === null ? failure : null"
      :skeleton-rows="4"
      testid="home-state"
      @retry="load"
    >
      <div
        class="fd-panel"
        data-testid="home-attention-panel"
      >
        <div class="fd-panel__head">
          <div>
            <div class="fd-panel__title">
              {{ heading }}
            </div>
            <div class="fd-panel__sub">
              Each row names the cause and the fix. Nothing here is decoration.
            </div>
          </div>
          <button
            type="button"
            class="fd-btn fd-btn--ghost fd-btn--sm"
            data-testid="home-refresh"
            @click="load"
          >
            Refresh
          </button>
        </div>

        <div
          class="fd-stack"
          data-testid="home-attention-list"
        >
          <AttentionRow
            v-for="item in attention"
            :key="item.id"
            :title="item.title"
            :why="item.why"
            :cause="item.cause"
            :fix="item.fix"
            :severity="item.severity"
            :action-label="item.action?.label ?? ''"
            :expanded="openRow === item.id"
            :testid="`home-attention-${item.id}`"
            @toggle="toggleRow(item.id)"
            @action="runAction(item)"
          />
        </div>
      </div>

      <HealthStrip
        :items="health"
        testid="home-health"
      />

      <div
        class="fd-panel"
        data-testid="home-quick-actions"
      >
        <div class="fd-between">
          <div>
            <div class="fd-panel__title">
              Quick actions
            </div>
            <div class="fd-panel__sub">
              What this wave can do from here.
            </div>
          </div>
          <div class="fd-cluster">
            <button
              v-for="action in quickActions"
              :key="action.id"
              type="button"
              class="fd-btn fd-btn--sm"
              :class="{ 'fd-btn--primary': action.primary }"
              :data-testid="`quick-${action.id}`"
              @click="action.run()"
            >
              {{ action.label }}
            </button>
          </div>
        </div>
      </div>

      <div
        class="fd-panel"
        data-testid="home-draft"
      >
        <div class="fd-between">
          <div>
            <div class="fd-panel__title">
              Draft
            </div>
            <div class="fd-panel__sub">
              <template v-if="draft.changeCount.value > 0">
                {{ draft.changeCount.value }} change{{ draft.changeCount.value === 1 ? '' : 's' }}
                sit between the running bridge and the draft. Revision
                {{ draft.revision.value }}.
              </template>
              <template v-else>
                The running configuration matches the draft. Revision
                {{ draft.revision.value }}.
              </template>
            </div>
          </div>
          <button
            type="button"
            class="fd-btn fd-btn--sm"
            data-testid="home-draft-review"
            :disabled="draft.changeCount.value === 0"
            @click="drawerOpen = true"
          >
            Review &amp; apply
          </button>
        </div>
      </div>
    </AsyncState>

    <DiffDrawer
      :open="drawerOpen"
      :changes="draft.changes.value"
      :revision="draft.revision.value"
      testid="home-diff"
      @close="drawerOpen = false"
    />
  </div>
</template>
