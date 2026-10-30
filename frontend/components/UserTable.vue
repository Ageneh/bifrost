<script setup lang="ts">
/**
 * The pairing history: a log, not an access list.
 *
 * Every Hue client shares one fixed username and one DTLS key, so a row
 * cannot be revoked — removing one would remove every client. The table says
 * what asked to pair and when, and nothing on it can be acted on.
 *
 * `devicetype` is a free string from an unauthenticated request body, so it
 * is rendered as text, truncated, and never as markup.
 */
import DataTable from '~/components/DataTable.vue'
import type { PairingRecord } from '~/lib/generated/PairingRecord'

withDefaults(
  defineProps<{
    records: PairingRecord[]
    loading?: boolean
  }>(),
  { loading: false },
)

const COLUMNS = [
  { key: 'devicetype', label: 'Device type' },
  { key: 'at', label: 'When' },
  { key: 'accepted', label: 'Result' },
]

/** The longest device type the table shows before cutting it. */
const MAX_DEVICETYPE = 40

function shortDeviceType(value: unknown): string {
  const text = String(value ?? '')
  return text.length > MAX_DEVICETYPE ? `${text.slice(0, MAX_DEVICETYPE - 1)}…` : text
}

/** RFC 3339 in the bridge's own clock, shown in the reader's locale. */
function when(value: unknown): string {
  const date = new Date(String(value ?? ''))
  return Number.isNaN(date.getTime()) ? String(value ?? '') : date.toLocaleString()
}

const rows = (records: PairingRecord[]) =>
  records.map((record, index) => ({ ...record, id: `${record.at}-${index}` }))
</script>

<template>
  <section
    class="fd-panel"
    data-testid="pairing-history"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          Pairing history
        </div>
        <div class="fd-panel__sub">
          A log, not an access list: every client shares one username and one key, so a row
          cannot be revoked on its own.
        </div>
      </div>
      <span
        class="fd-badge"
        data-testid="pairing-history-count"
      >
        {{ records.length }}
      </span>
    </div>

    <p
      v-if="!loading && records.length === 0"
      class="fd-muted"
      data-testid="pairing-history-empty"
    >
      Nothing has asked to pair yet.
    </p>

    <DataTable
      v-else
      :columns="COLUMNS"
      :rows="rows(records)"
      testid="pairing-history-table"
    >
      <template #cell-devicetype="{ value }">
        <span
          class="mono"
          data-testid="pairing-history-devicetype"
        >{{ shortDeviceType(value) }}</span>
      </template>
      <template #cell-at="{ value }">
        {{ when(value) }}
      </template>
      <template #cell-accepted="{ value }">
        <span
          class="fd-badge"
          :data-state="value ? 'ok' : 'warn'"
          :data-testid="value ? 'pairing-history-accepted' : 'pairing-history-refused'"
        >
          {{ value ? 'accepted' : 'refused' }}
        </span>
      </template>
    </DataTable>
  </section>
</template>
