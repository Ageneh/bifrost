/**
 * The live connection to `/bifrost/ws`.
 *
 * One socket for the whole shell. It reconnects with a capped backoff and
 * tells subscribers when a connection has been re-established, so a draft
 * apply that outlived a drop can be reconciled against the server.
 */
import { readonly, ref } from 'vue'
import { websocketUrl } from '~/lib/api'
import type { Update } from '~/lib/types'

export type SocketStatus = 'idle' | 'connecting' | 'open' | 'closed'

/** Reconnect delays; the last one repeats. */
const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 15000]

const status = ref<SocketStatus>('idle')
const lastMessageAt = ref<number | null>(null)
const connections = ref(0)

const updateListeners = new Set<(update: Update) => void>()
const reconnectListeners = new Set<() => void>()

let socket: WebSocket | null = null
let retry = 0
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let wanted = false

function scheduleReconnect(): void {
  if (!wanted || reconnectTimer !== null) {
    return
  }
  const delay = BACKOFF_MS[Math.min(retry, BACKOFF_MS.length - 1)] ?? 15000
  retry += 1
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    open()
  }, delay)
}

function open(): void {
  if (!wanted || socket !== null) {
    return
  }
  status.value = 'connecting'
  const current = new WebSocket(websocketUrl())
  socket = current

  current.onopen = () => {
    if (socket !== current) {
      return
    }
    const wasReconnect = retry > 0
    retry = 0
    status.value = 'open'
    connections.value += 1
    if (wasReconnect) {
      for (const listener of reconnectListeners) {
        listener()
      }
    }
  }

  current.onmessage = (event: MessageEvent) => {
    lastMessageAt.value = Date.now()
    if (typeof event.data !== 'string') {
      return
    }
    let update: Update
    try {
      update = JSON.parse(event.data) as Update
    }
    catch {
      return
    }
    for (const listener of updateListeners) {
      listener(update)
    }
  }

  current.onclose = () => {
    if (socket !== current) {
      return
    }
    socket = null
    status.value = 'closed'
    scheduleReconnect()
  }

  current.onerror = () => {
    // `onclose` follows and owns the retry
    current.close()
  }
}

export function useLiveSocket() {
  /** Start the socket; calling it twice is a no-op. */
  function connect(): void {
    wanted = true
    open()
  }

  /** Stop the socket and cancel any pending reconnect. */
  function disconnect(): void {
    wanted = false
    if (reconnectTimer !== null) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
    const current = socket
    socket = null
    status.value = 'idle'
    current?.close()
  }

  /** Listen for every update; returns the unsubscribe function. */
  function onUpdate(listener: (update: Update) => void): () => void {
    updateListeners.add(listener)
    return () => updateListeners.delete(listener)
  }

  /** Listen for a connection that was re-established after a drop. */
  function onReconnect(listener: () => void): () => void {
    reconnectListeners.add(listener)
    return () => reconnectListeners.delete(listener)
  }

  return {
    status: readonly(status),
    lastMessageAt: readonly(lastMessageAt),
    connections: readonly(connections),
    connect,
    disconnect,
    onUpdate,
    onReconnect,
  }
}
