<script setup lang="ts">
/**
 * A table that becomes one card per row below 600 px.
 *
 * The header text is repeated as `data-label` on each cell, so the card
 * layout keeps the column names instead of showing bare values.
 */
export interface TableColumn {
  key: string
  label: string
  /** Right-align and set in tabular numerals. */
  numeric?: boolean
}

withDefaults(
  defineProps<{
    columns: TableColumn[]
    rows: Array<Record<string, unknown>>
    rowKey?: string
    /** `cards` repeats the header in each cell below 600 px. */
    layout?: 'table' | 'cards'
    caption?: string
    testid?: string
  }>(),
  { rowKey: 'id', layout: 'cards', testid: 'data-table' },
)
</script>

<template>
  <table
    class="fd-table"
    :data-layout="layout"
    :data-testid="testid"
  >
    <caption
      v-if="caption"
      class="fd-label"
      style="text-align: left; padding-bottom: var(--fd-2)"
    >
      {{ caption }}
    </caption>
    <thead>
      <tr>
        <th
          v-for="column in columns"
          :key="column.key"
          scope="col"
        >
          {{ column.label }}
        </th>
      </tr>
    </thead>
    <tbody>
      <tr
        v-for="(row, index) in rows"
        :key="String(row[rowKey] ?? index)"
        :data-testid="`${testid}-row`"
      >
        <td
          v-for="column in columns"
          :key="column.key"
          :data-label="column.label"
          :class="{ 'fd-num': column.numeric }"
        >
          <slot
            :name="`cell-${column.key}`"
            :row="row"
            :value="row[column.key]"
          >
            {{ row[column.key] }}
          </slot>
        </td>
      </tr>
    </tbody>
  </table>
</template>
