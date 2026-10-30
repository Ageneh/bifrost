<script setup lang="ts">
/**
 * The six modes of the console, frozen for the whole programme.
 *
 * They are the only navigation there will be, so the order never changes and
 * a mode that has no screen yet still appears, disabled with the reason.
 */
import { computed } from 'vue'
import { useRoute } from 'vue-router'

export interface NavItem {
  key: string
  label: string
  path: string
  /** Shipped in a later wave; shown but not selectable. */
  pending?: boolean
}

/** The frozen set, in order. */
const items: NavItem[] = [
  { key: 'home', label: 'Home', path: '/' },
  { key: 'devices', label: 'Devices', path: '/devices' },
  { key: 'rooms', label: 'Rooms', path: '/rooms', pending: true },
  { key: 'entertainment', label: 'Entertainment', path: '/entertainment', pending: true },
  { key: 'pairing', label: 'Pairing', path: '/pairing', pending: true },
  { key: 'system', label: 'System', path: '/system/bridge' },
]

const route = useRoute()
const activePath = computed(() => route.path.replace(/\/+$/, '') || '/')
</script>

<template>
  <nav
    class="fd-nav"
    aria-label="Modes"
    data-testid="side-nav"
  >
    <NuxtLink
      v-for="item in items"
      :key="item.key"
      :to="item.path"
      class="fd-nav__item"
      :data-testid="`nav-${item.key}`"
      :data-state="activePath === item.path ? 'active' : item.pending ? 'pending' : 'idle'"
      :aria-current="activePath === item.path ? 'page' : undefined"
    >
      <svg
        class="fd-icon"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        aria-hidden="true"
      >
        <template v-if="item.key === 'home'">
          <circle
            cx="8"
            cy="8"
            r="5.5"
          />
          <circle
            cx="8"
            cy="8"
            r="1.5"
            fill="currentColor"
            stroke="none"
          />
        </template>
        <template v-else-if="item.key === 'devices'">
          <rect
            x="2.5"
            y="2.5"
            width="4.5"
            height="4.5"
            rx="1"
          />
          <rect
            x="9"
            y="2.5"
            width="4.5"
            height="4.5"
            rx="1"
          />
          <rect
            x="2.5"
            y="9"
            width="4.5"
            height="4.5"
            rx="1"
          />
          <rect
            x="9"
            y="9"
            width="4.5"
            height="4.5"
            rx="1"
          />
        </template>
        <template v-else-if="item.key === 'rooms'">
          <rect
            x="2.5"
            y="2.5"
            width="11"
            height="11"
            rx="1.5"
          />
          <path d="M6.5 13.5V9h3v4.5" />
        </template>
        <template v-else-if="item.key === 'entertainment'">
          <circle
            cx="8"
            cy="8"
            r="5.5"
          />
          <path
            d="M8 2.5a5.5 5.5 0 0 1 0 11"
            fill="currentColor"
            stroke="none"
          />
        </template>
        <template v-else-if="item.key === 'pairing'">
          <circle
            cx="8"
            cy="8"
            r="2"
          />
          <path d="M4.2 4.2a5.4 5.4 0 0 0 0 7.6M11.8 4.2a5.4 5.4 0 0 1 0 7.6" />
        </template>
        <template v-else>
          <circle
            cx="8"
            cy="8"
            r="2.2"
          />
          <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M12.6 3.4l-1.4 1.4M4.8 11.2l-1.4 1.4" />
        </template>
      </svg>
      <span class="fd-nav__label">{{ item.label }}</span>
      <span
        v-if="item.pending"
        class="fd-meta"
        aria-hidden="true"
      >soon</span>
    </NuxtLink>
  </nav>
</template>
