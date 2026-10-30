<script setup lang="ts">
/**
 * The residual LAN risk, said once and in place.
 *
 * The Hue API stays unauthenticated for compatibility, so the shell cannot
 * imply the bridge is locked down. This wording is fixed by the wave brief;
 * change it there first.
 */
</script>

<template>
  <div
    class="fd-notice"
    data-testid="lan-risk-notice"
    data-state="notice"
  >
    Hue apps on your network need no password. Anyone on this network can control your lights and
    add Zigbee devices.
  </div>
</template>
