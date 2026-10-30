<script setup lang="ts">
/**
 * One configuration field, rendered from metadata.
 *
 * Wave 3's screens describe their fields as data; nothing here knows what a
 * field means. No instance exists in wave 2 on purpose: the shell ships the
 * renderer, the screens come next.
 *
 * A secret is write-only. It renders as set or not set and never carries the
 * current value into the DOM; an empty submit keeps the stored secret, which
 * is what the API does with an absent field.
 */
import { computed } from 'vue'

export type FieldKind
  = | 'text'
    | 'number'
    | 'port'
    | 'select'
    | 'switch'
    | 'secret'
    | 'url'
    | 'mac'
    | 'ipv4'
    | 'timezone'
    | 'duration'

export interface FieldOption {
  value: string
  label: string
}

export interface FieldMeta {
  /** Dotted path, e.g. `bridge.http_port`. */
  path: string
  label: string
  kind: FieldKind
  hint?: string
  placeholder?: string
  options?: FieldOption[]
  /** Small unit after the input, e.g. `s` or `%`. */
  unit?: string
  min?: number
  max?: number
  /** A secret that is already stored; the input starts empty. */
  secretSet?: boolean
  readOnly?: boolean
  /**
   * What saving this field costs, shown in the label: `hot` takes effect at
   * once, `restart` restarts a listener or a backend, `restart required`
   * needs the whole process to restart.
   */
  blast?: 'hot' | 'restart' | 'restart required'
}

const props = withDefaults(
  defineProps<{
    field: FieldMeta
    modelValue?: string | number | boolean | null
    /** Findings for this path, from the last validation. */
    errors?: string[]
  }>(),
  { modelValue: null, errors: () => [] },
)

const emit = defineEmits<{ 'update:modelValue': [value: string | number | boolean | null] }>()

const testid = computed(() => `config-field-${props.field.path.replace(/[^a-zA-Z0-9]+/g, '-')}`)
const invalid = computed(() => props.errors.length > 0)

/** Problems the browser cannot express with an input type. */
const localError = computed(() => {
  const value = props.modelValue
  switch (props.field.kind) {
    case 'port': {
      const port = Number(value)
      return Number.isInteger(port) && port >= 1 && port <= 65535
        ? null
        : 'A port is a whole number from 1 to 65535.'
    }
    case 'mac':
      return value === '' || value === null || /^([0-9a-f]{2}:){5}[0-9a-f]{2}$/i.test(String(value))
        ? null
        : 'A MAC address looks like 00:11:22:33:44:55.'
    case 'ipv4':
      return isIpv4(value) ? null : 'An IPv4 address looks like 10.0.0.20.'
    case 'url':
      try {
        new URL(String(value))
        return null
      }
      catch {
        return 'This is not a URL.'
      }
    default:
      return null
  }
})

const message = computed(() => props.errors[0] ?? localError.value)
const state = computed(() =>
  message.value ? 'invalid' : props.field.readOnly ? 'readonly' : 'ok',
)

function update(event: Event): void {
  const target = event.target as HTMLInputElement | HTMLSelectElement
  const raw = props.field.kind === 'switch' ? (target as HTMLInputElement).checked : target.value
  emit('update:modelValue', normalise(raw))
}

/** Four decimal octets, each 0-255; leading zeros are allowed. */
function isIpv4(value: unknown): boolean {
  if (value === '' || value === null) {
    return true
  }
  const parts = String(value).split('.')
  return (
    parts.length === 4
    && parts.every(part => /^\d{1,3}$/.test(part) && Number(part) <= 255)
  )
}

function normalise(value: string | boolean): string | number | boolean | null {
  if (typeof value === 'boolean') {
    return value
  }
  if (props.field.kind === 'number' || props.field.kind === 'port' || props.field.kind === 'duration') {
    return value === '' ? null : Number(value)
  }
  return value
}
</script>

<template>
  <div
    class="fd-field"
    :data-testid="testid"
    :data-state="state"
  >
    <label
      class="fd-label"
      :for="`${testid}-input`"
    >
      {{ field.label }}<span
        v-if="field.blast"
        class="fd-meta blast"
        :data-testid="`${testid}-blast`"
      > · {{ field.blast }}</span>
    </label>

    <div
      v-if="field.kind === 'switch'"
      class="fd-cluster"
    >
      <input
        :id="`${testid}-input`"
        class="fd-switch"
        type="checkbox"
        role="switch"
        :checked="modelValue === true"
        :disabled="field.readOnly"
        :aria-invalid="invalid"
        @change="update"
      >
      <span class="fd-meta">{{ modelValue === true ? 'On' : 'Off' }}</span>
    </div>

    <select
      v-else-if="field.kind === 'select'"
      :id="`${testid}-input`"
      class="fd-input fd-select"
      :value="String(modelValue ?? '')"
      :disabled="field.readOnly"
      :aria-invalid="invalid"
      @change="update"
    >
      <option
        v-for="option in field.options ?? []"
        :key="option.value"
        :value="option.value"
      >
        {{ option.label }}
      </option>
    </select>

    <div
      v-else
      class="fd-cluster"
      style="flex-wrap: nowrap"
    >
      <input
        :id="`${testid}-input`"
        class="fd-input"
        :class="{ 'is-invalid': invalid }"
        :type="field.kind === 'secret' ? 'password' : field.kind === 'number' || field.kind === 'port' || field.kind === 'duration' ? 'number' : 'text'"
        :inputmode="field.kind === 'port' || field.kind === 'number' ? 'numeric' : undefined"
        :min="field.min"
        :max="field.max"
        :placeholder="field.kind === 'secret' && field.secretSet ? 'Set — leave empty to keep' : field.placeholder"
        :value="field.kind === 'secret' ? '' : String(modelValue ?? '')"
        :disabled="field.readOnly"
        :readonly="field.readOnly"
        :aria-invalid="invalid"
        autocomplete="off"
        @input="update"
      >
      <span
        v-if="field.unit"
        class="fd-meta"
      >{{ field.unit }}</span>
    </div>

    <span
      v-if="message"
      class="fd-meta"
      style="color: var(--fd-crit-text)"
      role="alert"
    >
      {{ message }}
    </span>
    <span
      v-else-if="field.hint"
      class="fd-meta"
    >{{ field.hint }}</span>
  </div>
</template>

<style scoped>
.blast {
  color: var(--fd-text-3);
  font-size: var(--fd-size-micro);
  letter-spacing: var(--fd-track-micro);
  text-transform: uppercase;
  white-space: nowrap;
}
</style>
