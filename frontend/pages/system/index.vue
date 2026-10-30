<script setup lang="ts">
/**
 * `/system` opens the first System tab.
 *
 * The rail links straight to Bridge; this route only exists so a typed or
 * remembered `/system` lands on a screen instead of the catch-all.
 */
import { onMounted } from 'vue'

const router = useRouter()

onMounted(() => {
  void router.replace('/system/bridge')
})
</script>

<template>
  <div
    class="fd-panel"
    data-testid="system-redirect"
  >
    <div class="fd-panel__sub">
      Opening Bridge &amp; network…
    </div>
  </div>
</template>
