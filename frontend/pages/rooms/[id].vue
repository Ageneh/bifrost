<script setup lang="ts">
/**
 * One room: what it is called, what is in it, and the scenes it holds.
 *
 * Membership is the zigbee2mqtt group. This screen drafts a new membership
 * into the configuration and apply sends the difference, so a click here
 * never touches the mesh. The name has three writers and the screen says
 * so: the configuration wins at import, and a rename restarts the backend
 * that owns the room.
 */
import { computed, onMounted, ref } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import DevicePicker from '~/components/DevicePicker.vue'
import ImpactList from '~/components/ImpactList.vue'
import { useDraft } from '~/composables/useDraft'
import { useRooms } from '~/composables/useRooms'
import { useToast } from '~/composables/useToast'

const route = useRoute()
const { loading, failure, refresh, byId, recall } = useRooms()
const draft = useDraft()
const toast = useToast()

const id = computed(() => String(route.params.id ?? ''))
const room = computed(() => byId(id.value))

const pickerOpen = ref(false)
const nameDraft = ref('')
const iconDraft = ref('')
/** The draft revision must be known before anything is written to it. */
const draftReady = ref(false)

/** The archetypes the Hue API knows; the room icon is one of them. */
const ICONS = [
  'living_room', 'kitchen', 'dining', 'bedroom', 'kids_bedroom', 'bathroom',
  'nursery', 'office', 'guest_room', 'hallway', 'toilet', 'staircase',
  'laundry_room', 'storage', 'closet', 'garage', 'gym', 'lounge', 'other',
]

/** The membership the draft holds for this room, or what the bridge has. */
const members = computed<string[]>(() => {
  const topic = room.value?.topic
  if (topic === undefined) {
    return []
  }
  const section = draft.document.value?.rooms as Record<string, unknown> | undefined
  const held = section?.[topic] as { devices?: string[] } | undefined
  if (Array.isArray(held?.devices)) {
    return held.devices
  }
  return room.value?.devices.map(device => device.ieee) ?? []
})

/** The devices in the drafted membership, with their names. */
const memberDevices = computed(() => {
  const known = room.value?.devices ?? []
  return members.value.map((ieee) => {
    const device = known.find(entry => entry.ieee === ieee)
    return { ieee, name: device?.name ?? ieee, supported: device?.supported ?? true }
  })
})

/** True while this room has edits waiting in the draft. */
const pending = computed(() => {
  const topic = room.value?.topic
  return topic !== undefined
    && draft.changes.value.some(change => change.path.startsWith(`rooms.${topic}`))
})

/** The consequences of applying this room's changes, named before apply. */
const impact = computed(() => {
  if (!pending.value) {
    return []
  }
  return [
    'Applying re-creates the zigbee2mqtt backend that owns this room, so the bridge reconnects and a running stream stops.',
    'Home Assistant entity names and automations built on this room’s name change with it.',
  ]
})

function editName(): void {
  nameDraft.value = room.value?.name ?? ''
}

async function saveName(): Promise<void> {
  const topic = room.value?.topic
  if (topic === undefined || nameDraft.value.trim() === '') {
    return
  }
  const saved = await draft.patch({ rooms: { [topic]: { name: nameDraft.value.trim() } } })
  toast[saved ? 'ok' : 'fail'](saved ? 'Rename drafted.' : draft.error.value ?? 'The rename was not saved.')
}

async function saveIcon(): Promise<void> {
  const topic = room.value?.topic
  if (topic === undefined) {
    return
  }
  const saved = await draft.patch({ rooms: { [topic]: { icon: iconDraft.value } } })
  toast[saved ? 'ok' : 'fail'](saved ? 'Icon drafted.' : draft.error.value ?? 'The icon was not saved.')
}

async function saveMembers(ieees: string[]): Promise<void> {
  const topic = room.value?.topic
  if (topic === undefined) {
    return
  }
  pickerOpen.value = false
  const saved = await draft.patch({ rooms: { [topic]: { devices: ieees } } })
  toast[saved ? 'ok' : 'fail'](
    saved ? 'Membership drafted. Apply to send it to the group.' : draft.error.value ?? 'The membership was not saved.',
  )
}

async function recallScene(sceneId: string): Promise<void> {
  const current = room.value
  if (current === undefined) {
    return
  }
  try {
    await recall(current, sceneId)
    toast.ok('Scene recalled.')
  }
  catch (error) {
    toast.fail(error instanceof Error ? error.message : 'The scene was not recalled.')
  }
}

onMounted(async () => {
  await Promise.all([refresh(), draft.load()])
  draftReady.value = true
  editName()
  iconDraft.value = room.value?.icon ?? 'other'
})
</script>

<template>
  <div
    class="room"
    data-testid="room-detail"
  >
    <AsyncState
      :loading="(loading && room === undefined) || !draftReady"
      :error="failure"
      :empty="!loading && room === undefined"
      empty-text="No room has that id. It may have been removed with its group."
      @retry="refresh"
    >
      <template v-if="room">
        <header class="fd-between">
          <div>
            <div class="fd-label">
              Room
            </div>
            <h1
              class="fd-panel__title"
              data-testid="room-name"
            >
              {{ room.name }}
            </h1>
            <div
              class="fd-meta fd-muted"
              data-testid="room-topic"
            >
              group {{ room.topic }}
            </div>
          </div>
          <span
            v-if="pending"
            class="fd-pill"
            data-testid="room-pending"
          >not applied</span>
        </header>

        <div
          class="fd-panel"
          data-testid="room-writers-note"
        >
          <div class="fd-panel__sub">
            A room’s name has three writers: this configuration, the Hue
            app’s own metadata, and the zigbee2mqtt group name. The
            configuration wins at import, so a name changed in the Hue app is
            overwritten the next time zigbee2mqtt reconnects.
          </div>
        </div>

        <section
          class="fd-panel"
          data-testid="room-identity"
        >
          <div class="fd-panel__body fd-stack">
            <div class="fd-panel__title">
              Name and icon
            </div>

            <label class="fd-field">
              <span class="fd-label">Name</span>
              <input
                v-model="nameDraft"
                class="fd-input"
                type="text"
                data-testid="room-name-input"
              >
            </label>

            <label class="fd-field">
              <span class="fd-label">Icon</span>
              <select
                v-model="iconDraft"
                class="fd-select"
                data-testid="room-icon"
              >
                <option
                  v-for="icon in ICONS"
                  :key="icon"
                  :value="icon"
                >
                  {{ icon }}
                </option>
              </select>
            </label>

            <div class="fd-cluster">
              <button
                type="button"
                class="fd-btn fd-btn--sm"
                data-testid="room-name-save"
                @click="saveName"
              >
                Save name
              </button>
              <button
                type="button"
                class="fd-btn fd-btn--sm"
                data-testid="room-icon-save"
                @click="saveIcon"
              >
                Save icon
              </button>
            </div>

            <ImpactList
              v-if="pending"
              :lines="impact"
              testid="room-impact"
            />
          </div>
        </section>

        <section
          class="fd-panel"
          data-testid="room-members"
        >
          <div class="fd-panel__body fd-stack">
            <div class="fd-between">
              <div class="fd-panel__title">
                Devices
              </div>
              <button
                type="button"
                class="fd-btn fd-btn--primary fd-btn--sm"
                data-testid="room-change-devices"
                @click="pickerOpen = true"
              >
                Choose devices
              </button>
            </div>

            <ul
              class="members"
              data-testid="room-member-list"
            >
              <li
                v-for="device in memberDevices"
                :key="device.ieee"
                class="members__item"
                data-testid="room-member"
              >
                <span>{{ device.name }}</span>
                <span class="fd-meta fd-muted">{{ device.ieee }}</span>
                <span
                  v-if="!device.supported"
                  class="fd-meta fd-muted"
                >not served by Bifrost</span>
              </li>
            </ul>

            <div
              v-if="memberDevices.length === 0"
              class="fd-muted"
              data-testid="room-members-empty"
            >
              This room is empty.
            </div>
          </div>
        </section>

        <section
          class="fd-panel"
          data-testid="room-scenes"
        >
          <div class="fd-panel__body fd-stack">
            <div class="fd-panel__title">
              Scenes
            </div>
            <div class="fd-panel__sub">
              Recall only. Scenes are authored in the Hue app.
            </div>

            <ul
              class="members"
              data-testid="room-scene-list"
            >
              <li
                v-for="scene in room.scenes"
                :key="scene.id"
                class="members__item"
                data-testid="room-scene"
                :data-active="scene.active"
              >
                <span>{{ scene.name }}</span>
                <span
                  v-if="scene.active"
                  class="fd-meta fd-muted"
                  data-testid="room-scene-active"
                >showing</span>
                <button
                  type="button"
                  class="fd-btn fd-btn--ghost fd-btn--sm"
                  data-testid="room-scene-recall"
                  @click="recallScene(scene.id)"
                >
                  Recall
                </button>
              </li>
            </ul>

            <div
              v-if="room.scenes.length === 0"
              class="fd-muted"
              data-testid="room-scenes-empty"
            >
              No scene in this room yet.
            </div>
          </div>
        </section>

        <DevicePicker
          :open="pickerOpen"
          :selected="members"
          only="light"
          title="Devices in this room"
          testid="room-picker"
          @close="pickerOpen = false"
          @confirm="saveMembers"
        />
      </template>
    </AsyncState>
  </div>
</template>

<style scoped>
.room {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.members {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--fd-1);
}
.members__item {
  display: flex;
  align-items: center;
  gap: var(--fd-2);
  justify-content: space-between;
  border-bottom: 1px solid var(--fd-line);
  padding-bottom: var(--fd-1);
}
</style>
