<script setup lang="ts">
/**
 * Where each segment sits, on a screen and in numbers.
 *
 * Numbers come first: drag is a follow-up, and a position typed here is the
 * same number the Hue app would write. The screen is a reading of those
 * numbers, not a second source: a dot moves because an input changed.
 *
 * A device that is offline keeps its position and is drawn as a hollow
 * marker, because a zone outlives the moment a bulb is unplugged.
 */
import { computed } from 'vue'
import type { ZoneMember } from '~/lib/entertainment'

export interface CanvasPosition {
  x: number
  y: number
  z: number
}

const props = defineProps<{
  members: ZoneMember[]
  /** The editable positions, by service, one per segment. */
  positions: Record<string, CanvasPosition[]>
  /** Read-only at a narrow width: the numbers are shown, not edited. */
  readonly?: boolean
  testid?: string
}>()

const emit = defineEmits<{
  move: [service: string, index: number, position: CanvasPosition]
}>()

interface Dot {
  service: string
  name: string
  index: number
  online: boolean
  position: CanvasPosition
}

/** One dot per segment, in member order and then segment order. */
const dots = computed<Dot[]>(() => props.members.flatMap((member) => {
  const held = props.positions[member.service] ?? []
  return held.map((position, index) => ({
    service: member.service,
    name: member.name,
    index,
    online: member.online,
    position,
  }))
}))

/** Where a dot sits on the canvas, in per cent of the box. */
function left(position: CanvasPosition): string {
  return `${Math.min(Math.max((position.x + 1) / 2, 0), 1) * 100}%`
}

function bottom(position: CanvasPosition): string {
  return `${Math.min(Math.max((position.y + 1) / 2, 0), 1) * 100}%`
}

function move(service: string, index: number, axis: 'x' | 'y' | 'z', value: string): void {
  const parsed = Number(value)
  if (value.trim() === '' || Number.isNaN(parsed)) {
    return
  }
  const current = props.positions[service]?.[index]
  if (current === undefined) {
    return
  }
  emit('move', service, index, { ...current, [axis]: parsed })
}
</script>

<template>
  <div
    class="canvas-wrap"
    :data-testid="testid ?? 'position-canvas'"
    :data-state="readonly ? 'read-only' : 'editable'"
  >
    <div
      class="canvas"
      aria-hidden="true"
    >
      <span class="canvas__centre" />
      <span
        v-for="dot in dots"
        :key="`${dot.service}-${dot.index}`"
        class="pos"
        :class="{ 'pos--off': !dot.online }"
        :data-testid="`position-dot-${dot.service}-${dot.index}`"
        :data-state="dot.online ? 'online' : 'offline'"
        :style="{ left: left(dot.position), bottom: bottom(dot.position) }"
        :title="`${dot.name}${dot.index > 0 ? ` #${dot.index + 1}` : ''}`"
      />
      <span class="canvas__axis canvas__axis--x">x</span>
      <span class="canvas__axis canvas__axis--y">y</span>
    </div>

    <div class="fd-stack">
      <div
        v-for="member in members"
        :key="member.service"
        class="member"
        :data-testid="`position-member-${member.service}`"
      >
        <div class="fd-between">
          <div class="fd-value">
            {{ member.name }}
          </div>
          <span
            class="fd-meta fd-muted"
            :data-state="member.online ? 'online' : 'offline'"
          >
            {{ member.online ? 'online' : 'offline' }}
          </span>
        </div>

        <div
          v-for="(position, index) in (positions[member.service] ?? [])"
          :key="`${member.service}-${index}`"
          class="row"
        >
          <span class="fd-meta fd-muted">
            segment {{ index + 1 }} of {{ Math.max(member.segments, index + 1) }}
          </span>
          <label
            v-for="axis in (['x', 'y', 'z'] as const)"
            :key="axis"
            class="fd-field row__field"
          >
            <span class="fd-label">{{ axis }}</span>
            <input
              class="fd-input fd-num"
              type="number"
              step="0.05"
              min="-1"
              max="1"
              :value="position[axis]"
              :disabled="readonly"
              :data-testid="`position-${member.service}-${index}-${axis}`"
              @change="move(member.service, index, axis, ($event.target as HTMLInputElement).value)"
            >
          </label>
        </div>

        <div
          v-if="(positions[member.service] ?? []).length === 0"
          class="fd-meta fd-muted"
        >
          No segment of this device is laid out yet.
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.canvas-wrap {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
}
.canvas {
  position: relative;
  aspect-ratio: 16 / 10;
  width: 100%;
  max-width: 420px;
  border: 1px solid var(--fd-line-strong);
  border-radius: var(--fd-r-inset);
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.03), transparent 60%),
    var(--fd-surface-1);
}
.canvas__centre {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 6px;
  height: 6px;
  margin: -3px 0 0 -3px;
  border-radius: 50%;
  background: var(--fd-surface-4);
}
.canvas__axis {
  position: absolute;
  font-size: var(--fd-size-micro);
  letter-spacing: var(--fd-track-micro);
  text-transform: uppercase;
  color: var(--fd-text-3);
}
.canvas__axis--x {
  right: var(--fd-2);
  bottom: var(--fd-1);
}
.canvas__axis--y {
  left: var(--fd-2);
  top: var(--fd-1);
}
.pos {
  position: absolute;
  width: 10px;
  height: 10px;
  margin: 0 0 -5px -5px;
  border-radius: 50%;
  background: var(--fd-accent);
  box-shadow: 0 0 0 2px var(--fd-bg);
}
.pos--off {
  background: transparent;
  border: 1px dashed var(--fd-text-3);
  box-shadow: none;
}
.member {
  display: flex;
  flex-direction: column;
  gap: var(--fd-1);
  border-bottom: 1px solid var(--fd-line);
  padding-bottom: var(--fd-2);
}
.row {
  display: flex;
  align-items: flex-end;
  gap: var(--fd-2);
  flex-wrap: wrap;
}
.row__field {
  width: 84px;
}
</style>
