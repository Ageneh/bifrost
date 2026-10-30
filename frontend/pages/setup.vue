<script setup lang="ts">
/**
 * First run, in seven steps.
 *
 * The password comes first and gates everything, so by the time this screen
 * appears the session exists. Every step saves its progress to the wizard's
 * own record beside the configuration draft — never to the draft itself, so
 * a half-finished setup is not "unapplied edits" and discarding a draft
 * cannot throw setup away. The draft is written once, at review.
 *
 * The entertainment step is absent on purpose: it arrives with the recipe
 * that fills it, and a step that does nothing is worse than no step.
 */
import { computed, onMounted, ref } from 'vue'
import { useApi } from '~/composables/useApi'
import { useDraft } from '~/composables/useDraft'
import { useToast } from '~/composables/useToast'
import { ApiError } from '~/lib/api'
import type { DiscoveredDevice, DiscoveredRoom } from '~/lib/generated'

interface Step {
  key: string
  label: string
  title: string
  why: string
}

/** The seven steps, in the plan's order. */
const STEPS: Step[] = [
  {
    key: 'identity',
    label: 'Bridge',
    title: 'What this bridge is called',
    why: 'The name is what the Hue app shows and what the certificate is issued for.',
  },
  {
    key: 'network',
    label: 'Network',
    title: 'Where it answers',
    why: 'The address the bridge holds, and the ports it listens on.',
  },
  {
    key: 'z2m',
    label: 'zigbee2mqtt',
    title: 'The zigbee2mqtt server',
    why: 'Bifrost speaks to your Zigbee network through zigbee2mqtt’s websocket.',
  },
  {
    key: 'discovery',
    label: 'Find devices',
    title: 'What is on the network',
    why: 'Ask the server what it can see. Nothing is imported yet.',
  },
  {
    key: 'lights',
    label: 'Lights',
    title: 'Which lights to serve',
    why: 'A light you leave out stays on the network and stays out of the Hue app.',
  },
  {
    key: 'rooms',
    label: 'Rooms',
    title: 'Which lights belong together',
    why: 'A room is a zigbee2mqtt group. This is skippable; skipping does not keep the wizard alive.',
  },
  {
    key: 'review',
    label: 'Review',
    title: 'Review and apply',
    why: 'This writes the configuration once and starts the bridge.',
  },
]

const { client } = useApi()
const draft = useDraft()
const toast = useToast()

const stepIndex = ref(0)
const busy = ref(false)
const failure = ref<string | null>(null)
const ready = ref(false)

const bridge = ref({
  name: 'Bifrost',
  mac: '',
  ipaddress: '',
  netmask: '255.255.255.0',
  gateway: '',
  http_port: 80,
  https_port: 443,
  timezone: 'UTC',
})

const serverName = ref('zigbee')
const serverUrl = ref('')
const serverToken = ref('')

const discovered = ref<{ devices: DiscoveredDevice[], rooms: DiscoveredRoom[] } | null>(null)
/** The lights the wizard will serve, by IEEE address. */
const chosen = ref<string[]>([])
/** The drafted membership, per group topic. */
const membership = ref<Record<string, string[]>>({})

const step = computed(() => STEPS[stepIndex.value] ?? STEPS[0]!)
const isFirst = computed(() => stepIndex.value === 0)
const isLast = computed(() => stepIndex.value === STEPS.length - 1)

const servable = computed(() => (discovered.value?.devices ?? []).filter(device => device.supported))

const failureMessage = computed(() => {
  const cause = failure.value
  return cause === null ? null : cause
})

function recordFailure(error: unknown, fallback: string): void {
  failure.value = error instanceof ApiError || error instanceof Error ? error.message : fallback
}

/** The partial configuration the wizard has collected. */
function values(): Record<string, unknown> {
  const z2m = serverUrl.value.trim() === ''
    ? {}
    : {
        [serverName.value.trim() || 'zigbee']: {
          url: serverUrl.value.trim(),
          ...(serverToken.value.trim() === '' ? {} : { token: serverToken.value.trim() }),
        },
      }

  const devices: Record<string, unknown> = {}
  for (const device of discovered.value?.devices ?? []) {
    devices[device.ieee] = { enabled: chosen.value.includes(device.ieee) }
  }

  const rooms: Record<string, unknown> = {}
  for (const [topic, ieees] of Object.entries(membership.value)) {
    rooms[topic] = { devices: ieees }
  }

  return { bridge: bridge.value, z2m, devices, rooms }
}

async function load(): Promise<void> {
  try {
    const progress = await client.wizard()
    const held = (progress.values ?? {}) as {
      bridge?: typeof bridge.value
      z2m?: Record<string, { url?: string, token?: string }>
      devices?: Record<string, { enabled?: boolean }>
      rooms?: Record<string, { devices?: string[] }>
    }

    if (held.bridge) {
      bridge.value = { ...bridge.value, ...held.bridge }
    }

    const [name, server] = Object.entries(held.z2m ?? {})[0] ?? []
    if (name !== undefined && server !== undefined) {
      serverName.value = name
      serverUrl.value = server.url ?? ''
      serverToken.value = server.token ?? ''
    }

    if (held.devices) {
      chosen.value = Object.entries(held.devices)
        .filter(([, settings]) => settings.enabled !== false)
        .map(([ieee]) => ieee)
    }

    if (held.rooms) {
      membership.value = Object.fromEntries(
        Object.entries(held.rooms).map(([topic, settings]) => [topic, settings.devices ?? []]),
      )
    }

    const at = STEPS.findIndex(entry => entry.key === progress.step)
    stepIndex.value = at >= 0 ? at : 0
  }
  catch (error) {
    recordFailure(error, 'The wizard could not read its progress.')
  }
  finally {
    ready.value = true
  }
}

/** Save the step the wizard is on, so a restart resumes here. */
async function save(): Promise<boolean> {
  try {
    await client.saveWizard({ step: step.value.key, values: values() as never })
    return true
  }
  catch (error) {
    recordFailure(error, 'The progress was not saved.')
    return false
  }
}

async function next(): Promise<void> {
  failure.value = null
  if (!isLast.value) {
    // the record names the step the wizard moves to, so a reload lands
    // where the user was, not one step behind
    stepIndex.value += 1
  }
  await save()
}

async function back(): Promise<void> {
  failure.value = null
  if (!isFirst.value) {
    stepIndex.value -= 1
    await save()
  }
}

async function discover(): Promise<void> {
  busy.value = true
  failure.value = null
  try {
    const result = await client.discovery({
      server: {
        url: serverUrl.value.trim(),
        ...(serverToken.value.trim() === '' ? {} : { token: serverToken.value.trim() }),
      } as never,
    })
    discovered.value = result
    // every servable light starts chosen; the user unticks what should stay
    // out, which is the common case in a mixed install
    if (chosen.value.length === 0) {
      chosen.value = result.devices.filter(device => device.supported).map(device => device.ieee)
    }
    for (const room of result.rooms) {
      membership.value[room.topic] ??= room.members.filter(ieee => chosen.value.includes(ieee))
    }
    await save()
  }
  catch (error) {
    recordFailure(error, 'The server did not answer.')
  }
  finally {
    busy.value = false
  }
}

/** The configuration review writes, built from what the wizard collected. */
function document(): Record<string, unknown> {
  const collected = values()
  return {
    bridge: collected.bridge,
    z2m: collected.z2m,
    devices: collected.devices,
    rooms: collected.rooms,
  }
}

/** The lines review shows before anything is written. */
const summary = computed(() => {
  const lines = [
    `Bridge “${bridge.value.name}” at ${bridge.value.ipaddress || 'no address yet'}`,
    serverUrl.value === '' ? 'No zigbee2mqtt server' : `zigbee2mqtt at ${serverUrl.value}`,
    `${chosen.value.length} light${chosen.value.length === 1 ? '' : 's'} served`,
    `${Object.keys(membership.value).length} room${Object.keys(membership.value).length === 1 ? '' : 's'}`,
  ]
  return lines
})

async function apply(): Promise<void> {
  busy.value = true
  failure.value = null
  try {
    await save()
    await draft.load()
    if (!(await draft.save(document() as never))) {
      failure.value = draft.error.value ?? 'The configuration was not accepted.'
      return
    }
    if (!(await draft.apply())) {
      failure.value = draft.error.value ?? 'The configuration was not applied.'
      return
    }
    toast.ok('Bifrost is configured. The bridge is starting.')
    // the hash only: a router push would drop the ingress prefix
    globalThis.location.hash = '#/'
  }
  catch (error) {
    recordFailure(error, 'The configuration was not applied.')
  }
  finally {
    busy.value = false
  }
}

function toggleLight(ieee: string, on: boolean): void {
  chosen.value = on ? [...chosen.value, ieee] : chosen.value.filter(entry => entry !== ieee)
}

function toggleMember(topic: string, ieee: string, on: boolean): void {
  const held = membership.value[topic] ?? []
  membership.value[topic] = on ? [...held, ieee] : held.filter(entry => entry !== ieee)
}

onMounted(load)
</script>

<template>
  <div
    class="wizard"
    data-testid="wizard"
  >
    <header class="fd-between wizard__head">
      <div>
        <div class="fd-label">
          First run · step {{ stepIndex + 1 }} of {{ STEPS.length }}
        </div>
        <h1 class="fd-panel__title">
          {{ step.title }}
        </h1>
        <div class="fd-panel__sub">
          {{ step.why }}
        </div>
      </div>
      <div
        class="fd-meta fd-muted"
        data-testid="wizard-step-count"
      >
        {{ stepIndex + 1 }} / {{ STEPS.length }}
      </div>
    </header>

    <nav
      class="fd-cluster wizard__steps"
      aria-label="Wizard steps"
      data-testid="wizard-steps"
    >
      <span
        v-for="(entry, index) in STEPS"
        :key="entry.key"
        class="fd-pill"
        :data-active="index === stepIndex"
        :data-done="index < stepIndex"
        :data-testid="`wizard-step-${entry.key}`"
      >{{ entry.label }}</span>
    </nav>

    <div
      v-if="!ready"
      class="fd-skeleton"
      style="height: 160px"
    />

    <section
      v-else
      class="fd-panel"
    >
      <div class="fd-panel__body fd-stack">
        <template v-if="step.key === 'identity'">
          <label class="fd-field">
            <span class="fd-label">Bridge name</span>
            <input
              v-model="bridge.name"
              class="fd-input"
              type="text"
              data-testid="wizard-bridge-name"
            >
          </label>
          <label class="fd-field">
            <span class="fd-label">MAC address</span>
            <input
              v-model="bridge.mac"
              class="fd-input"
              type="text"
              placeholder="00:11:22:33:44:55"
              data-testid="wizard-bridge-mac"
            >
          </label>
          <div class="fd-panel__sub">
            The bridge id and its certificate are derived from the MAC, so
            changing it later re-pairs every client.
          </div>
        </template>

        <template v-else-if="step.key === 'network'">
          <label class="fd-field">
            <span class="fd-label">Address</span>
            <input
              v-model="bridge.ipaddress"
              class="fd-input"
              type="text"
              placeholder="192.168.1.20"
              data-testid="wizard-ip"
            >
          </label>
          <label class="fd-field">
            <span class="fd-label">Netmask</span>
            <input
              v-model="bridge.netmask"
              class="fd-input"
              type="text"
              data-testid="wizard-netmask"
            >
          </label>
          <label class="fd-field">
            <span class="fd-label">Gateway</span>
            <input
              v-model="bridge.gateway"
              class="fd-input"
              type="text"
              data-testid="wizard-gateway"
            >
          </label>
          <label class="fd-field">
            <span class="fd-label">Timezone</span>
            <input
              v-model="bridge.timezone"
              class="fd-input"
              type="text"
              data-testid="wizard-timezone"
            >
          </label>
        </template>

        <template v-else-if="step.key === 'z2m'">
          <label class="fd-field">
            <span class="fd-label">Name</span>
            <input
              v-model="serverName"
              class="fd-input"
              type="text"
              data-testid="wizard-server-name"
            >
          </label>
          <label class="fd-field">
            <span class="fd-label">Websocket url</span>
            <input
              v-model="serverUrl"
              class="fd-input"
              type="text"
              placeholder="ws://192.168.1.10:8080/api"
              data-testid="wizard-server-url"
            >
          </label>
          <label class="fd-field">
            <span class="fd-label">Token</span>
            <input
              v-model="serverToken"
              class="fd-input"
              type="password"
              autocomplete="off"
              data-testid="wizard-server-token"
            >
          </label>
          <div class="fd-panel__sub">
            zigbee2mqtt 2.x needs a token; 1.x ignores it.
          </div>
        </template>

        <template v-else-if="step.key === 'discovery'">
          <div class="fd-cluster">
            <button
              type="button"
              class="fd-btn fd-btn--primary fd-btn--sm"
              :disabled="busy || serverUrl.trim() === ''"
              data-testid="wizard-discover"
              @click="discover"
            >
              Ask the server
            </button>
            <span
              class="fd-meta fd-muted"
              data-testid="wizard-discovery-count"
            >
              {{ discovered === null ? 'nothing asked yet' : `${discovered.devices.length} device(s)` }}
            </span>
          </div>

          <ul
            v-if="discovered"
            class="list"
            data-testid="wizard-discovery-list"
          >
            <li
              v-for="device in discovered.devices"
              :key="device.ieee"
              class="list__item"
              data-testid="wizard-discovered"
              :data-supported="device.supported"
            >
              <span>{{ device.friendly_name }}</span>
              <span class="fd-meta fd-muted">{{ device.ieee }}</span>
              <span
                v-if="!device.supported"
                class="fd-meta fd-muted"
              >not served by Bifrost</span>
            </li>
          </ul>
        </template>

        <template v-else-if="step.key === 'lights'">
          <div
            v-if="servable.length === 0"
            class="fd-muted"
            data-testid="wizard-lights-empty"
          >
            No servable light was found. Go back to discovery and ask the server.
          </div>
          <ul
            v-else
            class="list"
            data-testid="wizard-lights"
          >
            <li
              v-for="device in servable"
              :key="device.ieee"
              class="list__item"
            >
              <label class="fd-field fd-field--row">
                <input
                  type="checkbox"
                  :checked="chosen.includes(device.ieee)"
                  :data-testid="`wizard-light-${device.ieee}`"
                  @change="toggleLight(device.ieee, ($event.target as HTMLInputElement).checked)"
                >
                <span>{{ device.friendly_name }}</span>
              </label>
            </li>
          </ul>
        </template>

        <template v-else-if="step.key === 'rooms'">
          <div
            v-if="Object.keys(membership).length === 0"
            class="fd-muted"
            data-testid="wizard-rooms-empty"
          >
            The server has no group, so there is no room to fill. This step is
            skippable.
          </div>
          <div
            v-for="(ieees, topic) in membership"
            :key="topic"
            class="fd-stack"
            :data-testid="`wizard-room-${topic}`"
          >
            <div class="fd-label">
              {{ topic }}
            </div>
            <label
              v-for="device in servable"
              :key="`${topic}-${device.ieee}`"
              class="fd-field fd-field--row"
            >
              <input
                type="checkbox"
                :checked="ieees.includes(device.ieee)"
                :data-testid="`wizard-room-${topic}-${device.ieee}`"
                @change="toggleMember(topic, device.ieee, ($event.target as HTMLInputElement).checked)"
              >
              <span>{{ device.friendly_name }}</span>
            </label>
          </div>
        </template>

        <template v-else>
          <ul
            class="list"
            data-testid="wizard-summary"
          >
            <li
              v-for="line in summary"
              :key="line"
              class="list__item"
            >
              {{ line }}
            </li>
          </ul>
          <div class="fd-panel__sub">
            Applying writes the configuration once and starts the bridge.
            Nothing was written before this step.
          </div>
        </template>

        <div
          v-if="failureMessage"
          class="fd-alert"
          role="alert"
          data-testid="wizard-error"
        >
          {{ failureMessage }}
        </div>

        <div class="fd-cluster">
          <button
            type="button"
            class="fd-btn fd-btn--ghost fd-btn--sm"
            :disabled="isFirst || busy"
            data-testid="wizard-back"
            @click="back"
          >
            Back
          </button>
          <button
            v-if="!isLast"
            type="button"
            class="fd-btn fd-btn--primary fd-btn--sm"
            :disabled="busy"
            data-testid="wizard-next"
            @click="next"
          >
            Continue
          </button>
          <button
            v-else
            type="button"
            class="fd-btn fd-btn--primary fd-btn--sm"
            :disabled="busy"
            data-testid="wizard-apply"
            @click="apply"
          >
            Apply and start
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.wizard {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.wizard__head {
  flex-wrap: wrap;
  gap: var(--fd-2);
}
.wizard__steps {
  gap: var(--fd-1);
  flex-wrap: wrap;
}
.wizard__steps [data-active='true'] {
  background: var(--fd-accent-soft);
  color: var(--fd-accent-text);
}
.wizard__steps [data-done='true'] {
  color: var(--fd-muted);
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--fd-1);
}
.list__item {
  display: flex;
  gap: var(--fd-2);
  align-items: center;
  border-bottom: 1px solid var(--fd-line);
  padding-bottom: var(--fd-1);
}
</style>
