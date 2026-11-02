/**
 * The rooms, as the bridge holds them right now.
 *
 * A room is the zigbee2mqtt group the backend imported, so this is a read:
 * membership is drafted into the configuration and reaches the mesh at
 * apply, never from here.
 */
import { computed, readonly, ref } from 'vue'
import type { RoomEntry } from '~/lib/generated'
import { ApiError, api } from '~/lib/api'

const rooms = ref<RoomEntry[]>([])
const loading = ref(false)
const failure = ref<string | null>(null)

export function useRooms() {
  async function refresh(): Promise<void> {
    loading.value = true
    try {
      const list = await api.rooms()
      rooms.value = list.rooms
      failure.value = null
    }
    catch (error) {
      failure.value = error instanceof ApiError ? error.message : 'The rooms could not be read'
    }
    finally {
      loading.value = false
    }
  }

  /** The room with `id`, once it is loaded. */
  function byId(id: string): RoomEntry | undefined {
    return rooms.value.find(room => room.id === id)
  }

  /** Recall a scene, then read the rooms again so `active` follows. */
  async function recall(room: RoomEntry, sceneId: string): Promise<void> {
    await api.recallScene(room.id, sceneId)
    await refresh()
  }

  return {
    rooms,
    loading: readonly(loading),
    failure: readonly(failure),
    count: computed(() => rooms.value.length),
    refresh,
    byId,
    recall,
  }
}
