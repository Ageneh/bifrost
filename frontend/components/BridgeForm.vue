<script setup lang="ts">
/**
 * The bridge and network fields, each labelled with what saving it costs.
 *
 * The form edits a copy. Nothing reaches the server until Save, which sends
 * one merge patch holding only the fields that changed — a field that did
 * not change cannot be written by accident, and one round trip cannot race
 * another.
 *
 * The MAC is read-only on purpose: the bridge id and the certificate derive
 * from it, so changing it would make every paired client re-pair. That
 * decision belongs to the pairing work, not to a text field.
 */
import { computed, reactive, watch } from 'vue'
import ConfigField from './ConfigField.vue'
import type { FieldKind } from './ConfigField.vue'
import type { BridgeConfig } from '~/lib/generated/BridgeConfig'
import type { ValidationError } from '~/lib/generated/ValidationError'
import { timezoneOptions } from '~/lib/timezones'

type Blast = 'hot' | 'restart' | 'restart required'
type Value = string | number

interface FieldSpec {
  key: keyof BridgeConfig
  label: string
  kind: FieldKind
  blast: Blast
  hint: string
  readOnly?: boolean
  min?: number
  max?: number
}

/** Fixed order: identity, then the network, then the listeners. */
const FIELDS: FieldSpec[] = [
  {
    key: 'name',
    label: 'Bridge name',
    kind: 'text',
    blast: 'hot',
    hint: 'The name discovery advertises and clients show.',
  },
  {
    key: 'mac',
    label: 'MAC address',
    kind: 'mac',
    blast: 'restart required',
    hint: 'The bridge id and the certificate derive from the MAC. Changing it makes every Hue client re-pair, so it is read-only here.',
    readOnly: true,
  },
  {
    key: 'ipaddress',
    label: 'IP address',
    kind: 'ipv4',
    blast: 'restart',
    hint: 'Every listener binds to this address, and discovery advertises it.',
  },
  {
    key: 'netmask',
    label: 'Netmask',
    kind: 'ipv4',
    blast: 'restart',
    hint: 'The network the listeners bind to.',
  },
  {
    key: 'gateway',
    label: 'Gateway',
    kind: 'ipv4',
    blast: 'restart',
    hint: 'The network the listeners bind to.',
  },
  {
    key: 'timezone',
    label: 'Timezone',
    kind: 'select',
    blast: 'hot',
    hint: 'Schedules and timestamps use it.',
  },
  {
    key: 'http_port',
    label: 'HTTP port',
    kind: 'port',
    blast: 'restart',
    hint: 'Plain HTTP listener. Hue apps discover the bridge on it.',
    min: 1,
    max: 65535,
  },
  {
    key: 'https_port',
    label: 'HTTPS port',
    kind: 'port',
    blast: 'restart',
    hint: 'TLS listener.',
    min: 1,
    max: 65535,
  },
  {
    key: 'entm_port',
    label: 'Entertainment port',
    kind: 'port',
    blast: 'restart',
    hint: 'The stream listener Ambilight and Hue Sync connect to.',
    min: 1,
    max: 65535,
  },
]

const props = withDefaults(
  defineProps<{
    /** The running configuration: what the fields compare against. */
    active: BridgeConfig
    /** The draft when one exists, otherwise the active bridge settings. */
    model: BridgeConfig
    /** Findings for the draft; those on `bridge.*` are shown per field. */
    validation?: readonly ValidationError[]
    busy?: boolean
    testid?: string
  }>(),
  { validation: () => [], busy: false, testid: 'bridge-form' },
)

const emit = defineEmits<{
  save: [patch: Record<string, Value>]
}>()

const form = reactive<Record<string, Value>>({})

function reset(): void {
  for (const field of FIELDS) {
    const value = props.model[field.key]
    form[field.key] = typeof value === 'number' ? value : String(value ?? '')
  }
}

// the draft moves on after every save; the form follows it
watch(() => props.model, reset, { immediate: true })

const timezones = computed(() => timezoneOptions(String(form.timezone ?? '')))

const changed = computed(() => {
  const paths: Record<string, Value> = {}
  for (const field of FIELDS) {
    if (field.readOnly) {
      continue
    }
    const next = form[field.key]
    const before = props.model[field.key]
    if (normalise(next) !== normalise(before)) {
      paths[field.key] = next
    }
  }
  return paths
})

const changedCount = computed(() => Object.keys(changed.value).length)

function normalise(value: unknown): string | number | null {
  if (value === null || value === undefined || value === '') {
    return null
  }
  return typeof value === 'number' ? value : String(value)
}

/** Field-level findings, keyed by the field they belong to. */
function errorsFor(field: FieldSpec): string[] {
  return props.validation
    .filter(finding => finding.path === `bridge.${String(field.key)}`)
    .map(finding => finding.message)
}

/** The active value, shown under a field once the draft differs from it. */
function hintFor(field: FieldSpec): string {
  if (String(field.key) in changed.value) {
    return `Active value: ${String(props.active[field.key])}`
  }
  return field.hint
}

function optionsFor(field: FieldSpec) {
  return field.kind === 'select' ? timezones.value : undefined
}

/**
 * Whether saving moves the page off the address it is served from. The UI
 * rides the listener it changes, so this has to be said before the apply.
 */
const lockout = computed(() => {
  const moves: string[] = []
  if (String(form.ipaddress) !== String(props.active.ipaddress)) {
    moves.push('IP address')
  }
  if (normalise(form.http_port) !== normalise(props.active.http_port)) {
    moves.push('HTTP port')
  }
  if (normalise(form.https_port) !== normalise(props.active.https_port)) {
    moves.push('HTTPS port')
  }
  return moves
})

const lockoutUrl = computed(() => `http://${String(form.ipaddress)}:${String(form.http_port)}/`)

function save(): void {
  if (changedCount.value > 0) {
    emit('save', changed.value)
  }
}

/** Put every field back to the value it was opened with. */
function revert(): void {
  reset()
}

/** A cleared input is an empty string; numbers stay numbers for the API. */
function assign(key: keyof BridgeConfig, value: string | number | boolean | null): void {
  form[String(key)] = typeof value === 'number' ? value : typeof value === 'string' ? value : ''
}
</script>

<template>
  <div
    class="fd-stack"
    :data-testid="testid"
  >
    <div
      v-if="lockout.length > 0"
      class="fd-notice"
      data-testid="bridge-lockout"
      role="status"
    >
      Saving changes the {{ lockout.join(' and ') }} this page is served from. After the apply,
      reconnect at <span class="mono">{{ lockoutUrl }}</span>. If the new address does not answer,
      the previous listener is still up until the restart.
    </div>

    <div class="grid">
      <ConfigField
        v-for="field in FIELDS"
        :key="String(field.key)"
        :field="{
          path: `bridge.${String(field.key)}`,
          label: field.label,
          kind: field.kind,
          hint: hintFor(field),
          blast: field.blast,
          readOnly: field.readOnly,
          min: field.min,
          max: field.max,
          options: optionsFor(field),
        }"
        :model-value="form[String(field.key)]"
        :errors="errorsFor(field)"
        @update:model-value="value => assign(field.key, value)"
      />
    </div>

    <div class="fd-between">
      <div class="fd-meta">
        <template v-if="changedCount > 0">
          {{ changedCount }} field{{ changedCount === 1 ? '' : 's' }} changed. Nothing is saved
          until you save the draft.
        </template>
        <template v-else>
          The form matches the {{ model === active ? 'running' : 'draft' }} configuration.
        </template>
      </div>
      <div class="fd-cluster">
        <button
          type="button"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          data-testid="bridge-revert"
          :disabled="changedCount === 0 || busy"
          @click="revert"
        >
          Revert
        </button>
        <button
          type="button"
          class="fd-btn fd-btn--primary fd-btn--sm"
          data-testid="bridge-save"
          :disabled="changedCount === 0 || busy"
          @click="save"
        >
          Save to draft
        </button>
      </div>
    </div>

    <div
      class="fd-panel fd-panel--card"
      data-testid="bridge-blast-legend"
    >
      <div class="fd-label">
        What saving costs
      </div>
      <dl class="legend">
        <div>
          <dt><span class="fd-chip">hot</span></dt>
          <dd class="fd-meta">
            Takes effect at once. Nothing restarts.
          </dd>
        </div>
        <div>
          <dt><span class="fd-chip">restart</span></dt>
          <dd class="fd-meta">
            Restarts the listener that uses the field. Connections on it drop and come back.
          </dd>
        </div>
        <div>
          <dt><span class="fd-chip">restart required</span></dt>
          <dd class="fd-meta">
            The whole process restarts, about five seconds, and every client reconnects.
          </dd>
        </div>
      </dl>
    </div>
  </div>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--fd-4) var(--fd-5);
}
.legend {
  display: flex;
  flex-direction: column;
  gap: var(--fd-2);
  margin: var(--fd-2) 0 0;
}
.legend > div {
  display: grid;
  grid-template-columns: 120px minmax(0, 1fr);
  gap: var(--fd-3);
  align-items: baseline;
}
.legend dt {
  margin: 0;
}
.legend dd {
  margin: 0;
}
@media (max-width: 600px) {
  .grid {
    grid-template-columns: minmax(0, 1fr);
  }
  .legend > div {
    grid-template-columns: minmax(0, 1fr);
    gap: var(--fd-1);
  }
}
</style>
