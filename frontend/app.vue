<script setup lang="ts">
/**
 * The application root: read the session, then render the layout that
 * decides between the setup panel, the sign-in panel and the console.
 */
import { onMounted } from 'vue'
import { useApi } from '~/composables/useApi'

const { refresh } = useApi()

onMounted(async () => {
  await refresh()
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
