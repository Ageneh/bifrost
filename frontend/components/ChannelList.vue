<script setup lang="ts">
/**
 * The channels a zone defines, as the server derived them.
 *
 * This table is read-only on purpose. A channel is a colour in every frame,
 * and it exists because a segment sits somewhere: moving the segment moves
 * the channel. Editing channels here would be a second truth, and the two
 * would drift.
 */
import type { EntertainmentZoneChannel } from '~/lib/generated'

defineProps<{
  channels: EntertainmentZoneChannel[]
  /** The name of the device behind an entertainment service. */
  nameOf: (service: string) => string
  testid?: string
}>()
</script>

<template>
  <div
    class="fd-stack"
    :data-testid="testid ?? 'channel-list'"
  >
    <div class="fd-panel__sub">
      One channel per segment, in the order a frame must carry them. Computed
      from the members and their positions; there is nothing to edit here.
    </div>

    <table
      v-if="channels.length > 0"
      class="fd-table"
      data-layout="cards"
      data-testid="channel-table"
    >
      <thead>
        <tr>
          <th scope="col">
            Channel
          </th>
          <th scope="col">
            Device
          </th>
          <th scope="col">
            Segment
          </th>
          <th scope="col">
            X
          </th>
          <th scope="col">
            Y
          </th>
          <th scope="col">
            Z
          </th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="channel in channels"
          :key="channel.channel_id"
          :data-testid="`channel-row-${channel.channel_id}`"
        >
          <td
            data-label="Channel"
            class="fd-num"
          >
            {{ channel.channel_id }}
          </td>
          <td data-label="Device">
            <span
              v-for="member in channel.members"
              :key="`${member.service}-${member.index}`"
            >
              {{ nameOf(member.service) }}
            </span>
          </td>
          <td data-label="Segment">
            <span
              v-for="member in channel.members"
              :key="`segment-${member.service}-${member.index}`"
              class="fd-num"
            >{{ member.index + 1 }}</span>
          </td>
          <td
            data-label="X"
            class="fd-num"
          >
            {{ channel.position.x }}
          </td>
          <td
            data-label="Y"
            class="fd-num"
          >
            {{ channel.position.y }}
          </td>
          <td
            data-label="Z"
            class="fd-num"
          >
            {{ channel.position.z }}
          </td>
        </tr>
      </tbody>
    </table>

    <div
      v-else
      class="fd-muted"
      data-testid="channel-list-empty"
    >
      This zone has no channel yet. Add a light, and save.
    </div>
  </div>
</template>
