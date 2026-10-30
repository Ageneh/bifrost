<script setup lang="ts">
/**
 * What to do in each client while the window is open.
 *
 * Four rows, one per client, because "why won't the TV pair" is answered by
 * finding your own client in a list, not by reading prose. Each row names
 * the client, the screen it pairs from, and what the bridge expects to see.
 */

interface Instruction {
  key: string
  client: string
  where: string
  what: string
}

const ROWS: Instruction[] = [
  {
    key: 'hue-app',
    client: 'Hue app',
    where: 'Settings · Lights · Add light',
    what: 'Search while the window is open; the app sends its devicetype.',
  },
  {
    key: 'sync-desktop',
    client: 'Hue Sync (desktop)',
    where: 'Sync · Add bridge',
    what: 'Enter the bridge address, then press the button on screen when asked.',
  },
  {
    key: 'sync-box',
    client: 'Hue Sync box',
    where: 'The box pairs from the Hue app',
    what: 'Open the Hue app, add the Sync box, and press the link button when it asks.',
  },
  {
    key: 'tv-ambilight',
    client: 'TV Ambilight',
    where: 'The TV pairs from the Hue app',
    what: 'Add the TV in the Hue app, then press the link button while the TV searches.',
  },
]
</script>

<template>
  <section
    class="fd-panel"
    data-testid="pairing-instructions"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          What to do in each client
        </div>
        <div class="fd-panel__sub">
          The window has to be open when the client asks. A client that asks outside it is
          recorded as refused, and pairs on the next attempt.
        </div>
      </div>
    </div>

    <table
      class="fd-table"
      data-layout="cards"
    >
      <thead>
        <tr>
          <th scope="col">
            Client
          </th>
          <th scope="col">
            Where
          </th>
          <th scope="col">
            What the bridge expects
          </th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="row in ROWS"
          :key="row.key"
          :data-testid="`pairing-instruction-${row.key}`"
        >
          <td data-label="Client">
            {{ row.client }}
          </td>
          <td data-label="Where">
            {{ row.where }}
          </td>
          <td data-label="What the bridge expects">
            {{ row.what }}
          </td>
        </tr>
      </tbody>
    </table>
  </section>
</template>
