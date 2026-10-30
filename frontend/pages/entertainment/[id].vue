<script setup lang="ts">
/**
 * One zone: where its lights sit, what channels that makes, and its stream.
 *
 * The numbers are the truth and the canvas is their reading. Saving writes
 * the positions through the v2 `PUT` and reads the zone back, so the channel
 * table on screen is always the server's derivation of what was saved, never
 * the page's guess.
 *
 * "Save zone" is the primary action and Stop is the only other one. There is
 * no Start: a stream begins when the TV or the Sync app sends its first
 * frame.
 *
 * At 390 px the canvas is read-only: three numbers per segment on a phone is
 * not an edit anyone makes on purpose.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import ChannelList from '~/components/ChannelList.vue'
import DiagnosticsPanel from '~/components/DiagnosticsPanel.vue'
import PositionCanvas from '~/components/PositionCanvas.vue'
import SegmentMapPreview from '~/components/SegmentMapPreview.vue'
import StreamStatusCard from '~/components/StreamStatusCard.vue'
import { useEntertainment } from '~/composables/useEntertainment'
import { useStreams } from '~/composables/useStreams'
import { useToast } from '~/composables/useToast'
import type { CanvasPosition } from '~/components/PositionCanvas.vue'
import type { EntertainmentZone } from '~/lib/generated'
import type { ZoneService } from '~/composables/useEntertainment'

const route = useRoute()
const entertainment = useEntertainment()
const streams = useStreams()
const toast = useToast()

const id = computed(() => String(route.params.id ?? ''))
const zone = ref<EntertainmentZone | null>(null)
const loading = ref(true)
const failure = ref<string | null>(null)
const saving = ref(false)

/** The editable positions, by service, one per segment. */
const positions = ref<Record<string, CanvasPosition[]>>({})
/** The positions as last saved, so "dirty" is a comparison and not a flag. */
const saved = ref<Record<string, CanvasPosition[]>>({})

/** True while the window is too narrow to edit positions. */
const narrow = ref(false)
let media: MediaQueryList | null = null

const members = computed(() => (zone.value === null ? [] : entertainment.members(zone.value)))

const dirty = computed(
  () => JSON.stringify(positions.value) !== JSON.stringify(saved.value),
)

/** The zone's name, or the id until it is read. */
const title = computed(() => zone.value?.name ?? 'Zone')

/** The name of the device behind a service, for the channel table. */
function nameOf(service: string): string {
  return members.value.find(member => member.service === service)?.name ?? service
}

/** The name of the zone a stream belongs to, for the status card. */
function areaName(area: string): string {
  if (area === id.value) {
    return title.value
  }
  const known = entertainment.zones.value.find(entry => entry.id === area)
  return known?.name ?? 'An unknown area'
}

/** The stream on this zone, when one is running. */
const stream = computed(() => streams.streams().find(entry => entry.area === id.value))

/** A plain copy: a reactive proxy cannot be structured-cloned. */
function copy(held: Record<string, CanvasPosition[]>): Record<string, CanvasPosition[]> {
  return Object.fromEntries(
    Object.entries(held).map(([service, points]) => [service, points.map(point => ({ ...point }))]),
  )
}

function fromZone(read: EntertainmentZone): Record<string, CanvasPosition[]> {
  const held: Record<string, CanvasPosition[]> = {}
  for (const location of read.locations) {
    held[location.service] = location.positions.map(position => ({ ...position }))
  }
  return held
}

async function load(): Promise<void> {
  loading.value = true
  try {
    const read = await entertainment.zone(id.value)
    zone.value = read
    positions.value = fromZone(read)
    saved.value = copy(positions.value)
    failure.value = null
  }
  catch (error) {
    failure.value = error instanceof Error ? error.message : 'The zone could not be read.'
  }
  finally {
    loading.value = false
  }
}

/** Save the positions, then read the zone back for the derived channels. */
async function save(): Promise<void> {
  const current = zone.value
  if (current === null || narrow.value) {
    return
  }
  saving.value = true
  try {
    const services: ZoneService[] = current.locations.map(location => ({
      service: location.service,
      positions: positions.value[location.service] ?? [],
    }))
    await entertainment.save(current.id, services)
    await load()
    toast.ok('Zone saved.')
  }
  catch (error) {
    toast.fail(error instanceof Error ? error.message : 'The zone was not saved.')
  }
  finally {
    saving.value = false
  }
}

function move(service: string, index: number, position: CanvasPosition): void {
  const held = positions.value[service] ?? []
  positions.value = {
    ...positions.value,
    [service]: held.map((entry, at) => (at === index ? position : entry)),
  }
}

async function identify(service: string): Promise<void> {
  try {
    await entertainment.identify(service)
    toast.ok(`${nameOf(service)} is flashing.`)
  }
  catch (error) {
    toast.fail(error instanceof Error ? error.message : 'The device did not flash.')
  }
}

async function stop(): Promise<void> {
  try {
    await streams.stopStream(id.value)
    toast.ok('Stream stopped.')
  }
  catch (error) {
    toast.fail(error instanceof Error ? error.message : 'The stream did not stop.')
  }
}

onMounted(async () => {
  media = globalThis.matchMedia?.('(max-width: 600px)') ?? null
  narrow.value = media?.matches ?? false
  const onChange = (event: MediaQueryListEvent): void => {
    narrow.value = event.matches
  }
  media?.addEventListener('change', onChange)

  streams.start()
  await entertainment.load()
  await load()
})

onBeforeUnmount(() => {
  streams.stop()
})

// a zone that was reloaded because a save changed it must not look dirty
watch(zone, (read) => {
  if (read !== null) {
    positions.value = fromZone(read)
    saved.value = copy(positions.value)
  }
})
</script>

<template>
  <div
    class="zone"
    data-testid="zone-editor"
  >
    <AsyncState
      :loading="loading && zone === null"
      :error="failure"
      :empty="!loading && zone === null"
      empty-text="No zone has that id. It may have been removed from the Hue app."
      @retry="load"
    >
      <template v-if="zone">
        <header class="fd-between">
          <div>
            <NuxtLink
              to="/entertainment"
              class="fd-meta fd-muted"
              data-testid="zone-back"
            >Entertainment</NuxtLink>
            <h1
              class="fd-panel__title"
              data-testid="zone-name"
            >
              {{ zone.name }}
            </h1>
            <div class="fd-meta fd-muted">
              {{ zone.configuration_type }} · {{ zone.channels.length }} channel{{ zone.channels.length === 1 ? '' : 's' }}
              · {{ zone.status }}
            </div>
          </div>
          <div class="fd-cluster">
            <button
              v-if="stream"
              type="button"
              class="fd-btn fd-btn--ghost fd-btn--sm"
              data-testid="zone-stop"
              @click="stop"
            >
              Stop
            </button>
            <button
              type="button"
              class="fd-btn fd-btn--primary fd-btn--sm"
              :disabled="saving || narrow || !dirty"
              data-testid="zone-save"
              @click="save"
            >
              Save zone
            </button>
          </div>
        </header>

        <div class="columns">
          <div class="fd-stack">
            <section
              class="fd-panel"
              data-testid="zone-positions"
            >
              <div class="fd-panel__head">
                <div>
                  <div class="fd-panel__title">
                    Positions
                  </div>
                  <div class="fd-panel__sub">
                    One position per segment, in the Hue coordinate system: x
                    left to right, y bottom to top, z front to back.
                    <template v-if="narrow">
                      This window is too narrow to edit; widen it to change a
                      position.
                    </template>
                  </div>
                </div>
              </div>
              <div class="fd-panel__body fd-stack">
                <PositionCanvas
                  :members="members"
                  :positions="positions"
                  :readonly="narrow"
                  testid="zone-canvas"
                  @move="move"
                />

                <div
                  v-if="!narrow"
                  class="fd-cluster"
                >
                  <button
                    v-for="member in members"
                    :key="`identify-${member.service}`"
                    type="button"
                    class="fd-btn fd-btn--ghost fd-btn--sm"
                    :data-testid="`zone-identify-${member.service}`"
                    @click="identify(member.service)"
                  >
                    Identify {{ member.name }}
                  </button>
                </div>
              </div>
            </section>

            <section
              class="fd-panel"
              data-testid="zone-segments"
            >
              <div class="fd-panel__head">
                <div>
                  <div class="fd-panel__title">
                    Segment preview
                  </div>
                  <div class="fd-panel__sub">
                    The zone as a strip, left to right by x.
                  </div>
                </div>
              </div>
              <div class="fd-panel__body">
                <SegmentMapPreview
                  :channels="zone.channels"
                  testid="zone-segment-preview"
                />
              </div>
            </section>

            <section
              class="fd-panel"
              data-testid="zone-channels"
            >
              <div class="fd-panel__head">
                <div>
                  <div class="fd-panel__title">
                    Channels
                  </div>
                  <div class="fd-panel__sub">
                    Derived from the positions above; saved with the zone.
                  </div>
                </div>
              </div>
              <div class="fd-panel__body">
                <ChannelList
                  :channels="zone.channels"
                  :name-of="nameOf"
                  testid="zone-channel-list"
                />
              </div>
            </section>

            <DiagnosticsPanel
              :streams="streams.streams()"
              :name-of="areaName"
              testid="zone-diagnostics"
            />
          </div>

          <aside class="fd-stack">
            <StreamStatusCard
              :streams="streams.streams()"
              :name-of="areaName"
              :max-streams="entertainment.capabilities.value?.max_streams ?? 0"
              :failure="streams.failure.value"
              testid="zone-streams"
              @stop="stop"
            />

            <section
              class="fd-panel"
              data-testid="zone-note"
            >
              <div class="fd-panel__sub">
                The TV or the Sync app starts the stream. A zone saved here is
                the same resource the Hue app edits, so a change made there
                shows up here on the next read.
              </div>
            </section>
          </aside>
        </div>
      </template>
    </AsyncState>
  </div>
</template>

<style scoped>
.zone {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.columns {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr);
  gap: var(--fd-3);
  align-items: start;
}
@media (max-width: 900px) {
  .columns {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
