<script setup lang="ts">
/**
 * The three filters over the device table: what is typed into the search
 * box, which server, and whether Bifrost serves the device.
 *
 * The state lives in the page, not here, so the URL-less reload of the
 * socket never resets a filter a person set.
 */
import type { ServerInfo } from '~/lib/generated'

withDefaults(
  defineProps<{
    servers: ServerInfo[]
    server?: string
    supported?: 'all' | 'yes' | 'no'
    search?: string
  }>(),
  { server: '', supported: 'all', search: '' },
)

const emit = defineEmits<{
  'update:search': [value: string]
  'update:server': [value: string]
  'update:supported': [value: 'all' | 'yes' | 'no']
}>()
</script>

<template>
  <div
    class="fd-cluster filter-bar"
    data-testid="device-filter-bar"
  >
    <input
      class="fd-input"
      type="search"
      placeholder="Search name, model or address"
      aria-label="Search devices"
      data-testid="device-search"
      :value="search"
      @input="emit('update:search', ($event.target as HTMLInputElement).value)"
    >

    <select
      v-if="servers.length > 1"
      class="fd-select"
      aria-label="Server"
      data-testid="device-server-filter"
      :value="server"
      @change="emit('update:server', ($event.target as HTMLSelectElement).value)"
    >
      <option value="">
        Every server
      </option>
      <option
        v-for="entry in servers"
        :key="entry.name"
        :value="entry.name"
      >
        {{ entry.name }}{{ entry.online ? '' : ' (not running)' }}
      </option>
    </select>

    <select
      class="fd-select"
      aria-label="Mapping"
      data-testid="device-supported-filter"
      :value="supported"
      @change="emit('update:supported', ($event.target as HTMLSelectElement).value as 'all' | 'yes' | 'no')"
    >
      <option value="all">
        Mapped and unmapped
      </option>
      <option value="yes">
        Mapped only
      </option>
      <option value="no">
        Not mapped only
      </option>
    </select>
  </div>
</template>

<style scoped>
.filter-bar {
  gap: var(--fd-2);
  flex-wrap: wrap;
  align-items: center;
}
.filter-bar .fd-input {
  flex: 1 1 220px;
  min-width: 0;
}
</style>
