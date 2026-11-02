<script setup lang="ts">
/**
 * Rooms: the groups zigbee2mqtt holds, and what is in them.
 *
 * A room is not created here. It is a zigbee2mqtt group, so it arrives when
 * the group does; this screen decides what belongs to it and what it is
 * called. Nothing on this page reaches the mesh until the draft is applied.
 */
import { computed, onMounted } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import { useDraft } from '~/composables/useDraft'
import { useRooms } from '~/composables/useRooms'

const { rooms, loading, failure, count, refresh } = useRooms()
const draft = useDraft()

/** The drafted membership for a room, when the draft holds one. */
function drafted(topic: string): string[] | null {
  const section = draft.document.value?.rooms as Record<string, unknown> | undefined
  const room = section?.[topic] as { devices?: string[] } | undefined
  return Array.isArray(room?.devices) ? room.devices : null
}

/** The devices a room would hold once the draft is applied. */
function memberCount(topic: string, actual: number): number {
  return drafted(topic)?.length ?? actual
}

/** True while this room has edits waiting in the draft. */
function pending(topic: string): boolean {
  return draft.changes.value.some(change => change.path.startsWith(`rooms.${topic}`))
}

const anyPending = computed(() => draft.changes.value.some(change => change.path.startsWith('rooms.')))

onMounted(async () => {
  await Promise.all([refresh(), draft.load()])
})
</script>

<template>
  <div
    class="rooms"
    data-testid="rooms-view"
  >
    <header class="fd-between">
      <div>
        <div class="fd-label">
          Rooms
        </div>
        <h1 class="fd-panel__title">
          What belongs where
        </h1>
        <div
          class="fd-meta fd-muted"
          data-testid="rooms-count"
        >
          {{ count }} room{{ count === 1 ? '' : 's' }}
        </div>
      </div>
    </header>

    <AsyncState
      :loading="loading"
      :error="failure"
      :empty="!loading && count === 0"
      empty-text="No room yet. A room is a zigbee2mqtt group; create the group and it appears here."
      @retry="refresh"
    >
      <div
        class="fd-stack"
        data-testid="rooms-list"
      >
        <div
          v-for="room in rooms"
          :key="room.id"
          class="fd-panel room"
          data-testid="room-row"
          :data-topic="room.topic"
        >
          <div class="fd-between">
            <div>
              <div
                class="fd-panel__title"
                data-testid="room-row-name"
              >
                {{ room.name }}
              </div>
              <div class="fd-meta fd-muted">
                <span data-testid="room-row-icon">{{ room.icon }}</span>
                ·
                <span data-testid="room-row-devices">
                  {{ memberCount(room.topic, room.devices.length) }}
                  device{{ memberCount(room.topic, room.devices.length) === 1 ? '' : 's' }}
                </span>
                ·
                <span data-testid="room-row-scenes">{{ room.scenes.length }} scene{{ room.scenes.length === 1 ? '' : 's' }}</span>
              </div>
            </div>
            <div class="fd-cluster">
              <span
                v-if="pending(room.topic)"
                class="fd-pill"
                data-testid="room-row-pending"
              >not applied</span>
              <NuxtLink
                class="fd-btn fd-btn--sm"
                :to="`/rooms/${room.id}`"
                data-testid="room-open"
              >
                Open
              </NuxtLink>
            </div>
          </div>
        </div>
      </div>
    </AsyncState>

    <div
      v-if="anyPending"
      class="fd-panel"
      data-testid="rooms-draft-note"
    >
      <div class="fd-panel__sub">
        Membership changes sit in the draft. Nothing reaches the mesh until
        you apply, and a room whose group write fails is named in the result.
      </div>
    </div>
  </div>
</template>

<style scoped>
.rooms {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.room {
  padding: var(--fd-3);
}
</style>
