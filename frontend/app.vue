<script setup lang="ts">
/**
 * The application root: read the session, then keep one live socket open for
 * as long as there is a session to use it with.
 */
import { onBeforeUnmount, onMounted, watch } from 'vue'
import { useApi } from '~/composables/useApi'
import { useDraft } from '~/composables/useDraft'
import { useLiveSocket } from '~/composables/useLiveSocket'

const { refresh, authenticated } = useApi()
const socket = useLiveSocket()
const draft = useDraft()

let stopReconnect: (() => void) | null = null

function startLive(): void {
  socket.connect()
  stopReconnect ??= socket.onReconnect(() => {
    void draft.reconcileAfterReconnect()
  })
}

watch(
  authenticated,
  (signedIn) => {
    if (signedIn) {
      startLive()
    }
    else {
      socket.disconnect()
    }
  },
)

onMounted(async () => {
  await refresh()
  if (authenticated.value) {
    startLive()
  }
})

onBeforeUnmount(() => {
  stopReconnect?.()
  stopReconnect = null
  socket.disconnect()
})
</script>

<template>
  <div class="app">
    <NuxtLayout>
      <NuxtPage />
    </NuxtLayout>
    <ToastHost />
  </div>
</template>

<style scoped>
.app {
  min-height: 100%;
}
</style>
