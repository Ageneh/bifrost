<script setup lang="ts">
/**
 * Home: what the bridge is doing right now.
 *
 * The shell holds no configuration fields yet, so this view reports state
 * only — the bridge identity, the zigbee2mqtt servers it knows, the services
 * it runs and what the draft holds. Every number comes from the API or the
 * live socket, never from a placeholder.
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import AttentionRow from '~/components/AttentionRow.vue'
import DataTable from '~/components/DataTable.vue'
import StatTile from '~/components/StatTile.vue'
import StatusPill from '~/components/StatusPill.vue'
import { useApi } from '~/composables/useApi'
import { useDraft } from '~/composables/useDraft'
import { useLiveSocket } from '~/composables/useLiveSocket'
import type { AppConfig } from '~/lib/generated/AppConfig'
import type { Service } from '~/lib/generated/Service'

const { client } = useApi()
const socket = useLiveSocket()
const draft = useDraft()

const config = ref<AppConfig | null>(null)
const services = ref<Service[]>([])
const loading = ref(true)
const failure = ref<string | null>(null)

const columns = [
  { key: 'name', label: 'Service' },
  { key: 'state', label: 'State' },
]

const serverCount = computed(() => Object.keys(config.value?.z2m ?? {}).length)
const running = computed(() => services.value.filter(service => service.state === 'Running').length)
const failed = computed(() => services.value.filter(service => service.state === 'Failed'))

function stateOf(value: unknown): 'ok' | 'warn' | 'crit' | 'idle' {
  switch (value) {
    case 'Running':
      return 'ok'
    case 'Failed':
      return 'crit'
    case 'Starting':
    case 'Stopping':
    case 'Configured':
      return 'warn'
    default:
      return 'idle'
  }
}

async function load(): Promise<void> {
  loading.value = true
  failure.value = null
  try {
    const [active, list] = await Promise.all([client.activeConfig(), client.services()])
    // the active configuration is redacted but otherwise the same document
    config.value = active as unknown as AppConfig
    services.value = Object.values(list.services)
  }
  catch (cause) {
    failure.value = cause instanceof Error ? cause.message : 'Could not read the bridge state.'
  }
  finally {
    loading.value = false
  }
}

let stopUpdates: (() => void) | null = null

onMounted(async () => {
  stopUpdates = socket.onUpdate((update) => {
    if ('AppConfig' in update) {
      config.value = update.AppConfig
    }
    else if ('ServiceUpdate' in update) {
      const next = update.ServiceUpdate
      const index = services.value.findIndex(service => service.id === next.id)
      if (index >= 0) {
        services.value[index] = next
      }
      else {
        services.value = [...services.value, next]
      }
    }
  })
  await load()
  await draft.load()
})

onBeforeUnmount(() => {
  stopUpdates?.()
  stopUpdates = null
})
</script>

<template>
  <div
    class="fd-stack"
    data-testid="home-view"
  >
    <div
      class="fd-panel"
      data-testid="home-telemetry"
    >
      <div class="fd-panel__head">
        <div>
          <div class="fd-panel__title">
            Bridge
          </div>
          <div class="fd-panel__sub">
            State read from the running process.
          </div>
        </div>
        <StatusPill
          :label="failure ? 'unreachable' : loading ? 'reading' : 'reachable'"
          :state="failure ? 'crit' : loading ? 'warn' : 'ok'"
          testid="home-bridge-status"
        />
      </div>
      <div class="fd-readouts">
        <StatTile
          label="zigbee2mqtt"
          :value="serverCount"
          unit="servers"
          testid="stat-servers"
        />
        <StatTile
          label="Services"
          :value="`${running}/${services.length}`"
          unit="running"
          testid="stat-services"
        />
        <StatTile
          label="Draft"
          :value="draft.changeCount.value"
          unit="changes"
          testid="stat-draft"
        />
        <StatTile
          label="Revision"
          :value="draft.revision.value"
          testid="stat-revision"
        />
      </div>
    </div>

    <AttentionRow
      v-for="service in failed"
      :key="service.id"
      :title="`${service.name} failed`"
      why="The service manager reported a failure. Its log has the reason."
      severity="crit"
      testid="home-attention"
    >
      <StatusPill
        label="failed"
        state="crit"
        testid="attention-status"
      />
    </AttentionRow>

    <div class="fd-panel fd-panel--flush">
      <div
        class="fd-panel__body fd-between"
        style="border-bottom: 1px solid var(--fd-line)"
      >
        <div>
          <div class="fd-panel__title">
            Services
          </div>
          <div class="fd-panel__sub">
            Everything the bridge is running, live.
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
      <div class="fd-panel__body">
        <AsyncState
          :loading="loading"
          :error="failure"
          :empty="services.length === 0"
          empty-text="No services are registered."
          testid="home-services-state"
          @retry="load"
        >
          <DataTable
            :columns="columns"
            :rows="services as unknown as Array<Record<string, unknown>>"
            testid="home-services"
          >
            <template #cell-state="{ row }">
              <StatusPill
                :label="String(row.state)"
                :state="stateOf(row.state)"
                testid="service-state"
              />
            </template>
          </DataTable>
        </AsyncState>
      </div>
    </div>

    <div
      class="fd-panel"
      data-testid="home-placeholder"
    >
      <div class="fd-panel__title">
        Configuration screens
      </div>
      <div class="fd-panel__sub">
        The shell is wired to the bridge; the screens that edit it arrive in the next wave. Until
        then the draft bar shows what is pending and can apply it.
      </div>
    </div>
  </div>
</template>
