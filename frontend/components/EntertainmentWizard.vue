<script setup lang="ts">
/**
 * The recipe, carried out: pick the lights, name the zone, check, create.
 *
 * The zone this creates is the one the editor edits — same resource, same
 * rules. The only thing the recipe decides is what to call it and which
 * devices go in; the positions start empty and the server lays them out.
 *
 * The overlap check runs on the selection, not after the write: a light in
 * two zones is refused here, with both names, before the Hue app ever sees
 * it.
 */
import { computed, ref, watch } from 'vue'
import PreflightChecklist from '~/components/PreflightChecklist.vue'
import { useEntertainment } from '~/composables/useEntertainment'
import { useToast } from '~/composables/useToast'

const props = defineProps<{
  open: boolean
  /** The recipe's name, used as the zone name until the user changes it. */
  suggestion: string
  /** What the recipe is for, shown above the device list. */
  title: string
  /** The pairing step a Sync box needs after its zone exists. */
  pairing?: boolean
}>()

const emit = defineEmits<{
  close: []
  created: [id: string]
}>()

const entertainment = useEntertainment()
const toast = useToast()

const chosen = ref<string[]>([])
const name = ref('')
const busy = ref(false)
const failure = ref<string | null>(null)

const overlaps = computed(() => entertainment.overlaps(chosen.value))
const canCreate = computed(
  () => entertainment.ready.value
    && chosen.value.length > 0
    && name.value.trim() !== ''
    && overlaps.value.length === 0,
)

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return
    }
    chosen.value = []
    name.value = props.suggestion
    failure.value = null
  },
)

function toggle(id: string, on: boolean): void {
  chosen.value = on ? [...chosen.value, id] : chosen.value.filter(entry => entry !== id)
}

async function create(): Promise<void> {
  if (!canCreate.value) {
    return
  }
  busy.value = true
  failure.value = null
  try {
    const id = await entertainment.create(name.value.trim(), chosen.value)
    toast.ok('Zone created.')
    emit('created', id)
  }
  catch (error) {
    failure.value = error instanceof Error ? error.message : 'The zone was not created.'
    toast.fail(failure.value)
  }
  finally {
    busy.value = false
  }
}
</script>

<template>
  <div
    v-if="open"
    class="scrim"
    data-testid="entertainment-wizard"
  >
    <div
      class="fd-modal"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
    >
      <div class="fd-panel__head">
        <div>
          <div class="fd-label">
            Recipe
          </div>
          <div class="fd-panel__title">
            {{ title }}
          </div>
          <div class="fd-panel__sub">
            A zone the editor can change afterwards. Nothing streams until the TV
            or the app sends its first frame.
          </div>
        </div>
        <button
          type="button"
          class="fd-btn fd-btn--ghost fd-btn--sm"
          data-testid="entertainment-wizard-cancel"
          @click="emit('close')"
        >
          Cancel
        </button>
      </div>

      <div class="fd-panel__body fd-stack">
        <label class="fd-field">
          <span class="fd-label">Zone name</span>
          <input
            v-model="name"
            class="fd-input"
            type="text"
            data-testid="entertainment-wizard-name"
          >
        </label>

        <div class="fd-label">
          Lights
        </div>
        <ul
          class="devices"
          data-testid="entertainment-wizard-devices"
        >
          <li
            v-for="device in entertainment.devices.value"
            :key="device.id"
            class="devices__item"
            :data-testid="`entertainment-wizard-device-${device.id}`"
          >
            <label class="fd-field fd-field--row">
              <input
                type="checkbox"
                :checked="chosen.includes(device.id)"
                :data-testid="`entertainment-wizard-pick-${device.id}`"
                @change="toggle(device.id, ($event.target as HTMLInputElement).checked)"
              >
              <span>{{ device.name }}</span>
            </label>
            <span class="fd-meta fd-muted">
              {{ device.segments }} segment{{ device.segments === 1 ? '' : 's' }}
            </span>
          </li>
        </ul>

        <div
          v-if="overlaps.length > 0"
          class="fd-alert"
          data-testid="entertainment-wizard-overlap"
        >
          <div
            v-for="overlap in overlaps"
            :key="`${overlap.light}-${overlap.zoneId}`"
          >
            {{ overlap.name }} is already in {{ overlap.zone }}. A light belongs
            to one zone.
          </div>
        </div>

        <PreflightChecklist
          :rows="entertainment.preflight.value"
          testid="entertainment-wizard-preflight"
        />

        <div
          v-if="pairing"
          class="fd-panel__sub"
          data-testid="entertainment-wizard-pairing"
        >
          After the zone exists, pair the Sync box on the Pairing screen: the
          box has to be paired before it will start a stream.
        </div>

        <div
          v-if="failure"
          class="fd-alert"
          role="alert"
          data-testid="entertainment-wizard-error"
        >
          {{ failure }}
        </div>

        <div class="fd-cluster">
          <button
            type="button"
            class="fd-btn fd-btn--primary fd-btn--sm"
            :disabled="busy || !canCreate"
            data-testid="entertainment-wizard-create"
            @click="create"
          >
            Create zone
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  padding: var(--fd-4);
  background: rgba(4, 5, 7, 0.72);
  z-index: 50;
  overflow: auto;
}
.devices {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--fd-1);
}
.devices__item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--fd-2);
  border-bottom: 1px solid var(--fd-line);
  padding-bottom: var(--fd-1);
}
</style>
