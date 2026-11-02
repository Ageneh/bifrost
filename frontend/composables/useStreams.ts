/**
 * The entertainment streams, polled once a second while a page shows them.
 *
 * The socket says when the set of streams changes; it never carries a frame.
 * The rates are measured between two polls, so the poll is what makes them
 * real — and a page that is not showing the streams does not poll at all.
 */
import { readonly, ref } from 'vue'
import type { EntertainmentStatus, StreamStats } from '~/lib/generated'
import { api } from '~/lib/api'

/** The interval the server measures its rates over. */
const POLL_MS = 1000

const status = ref<EntertainmentStatus | null>(null)
const failure = ref<string | null>(null)

let timer: ReturnType<typeof setInterval> | null = null
let inFlight = false

async function poll(): Promise<void> {
  // a slow answer must not stack up behind the next tick
  if (inFlight) {
    return
  }
  inFlight = true
  try {
    status.value = await api.entertainmentStatus()
    failure.value = null
  }
  catch (cause) {
    failure.value = cause instanceof Error ? cause.message : 'Could not read the streams.'
  }
  finally {
    inFlight = false
  }
}

export function useStreams() {
  /** Begin polling; calling it twice starts one timer. */
  function start(): void {
    if (timer !== null) {
      return
    }
    void poll()
    timer = setInterval(() => void poll(), POLL_MS)
  }

  function stop(): void {
    if (timer !== null) {
      clearInterval(timer)
      timer = null
    }
  }

  /** End the stream on one area. */
  async function stopStream(area: string): Promise<void> {
    await api.stopStream(area)
    await poll()
  }

  /** A stream that is running, by its area, which is what the stop names. */
  function streams(): StreamStats[] {
    return status.value?.streams ?? []
  }

  return {
    status: readonly(status),
    failure: readonly(failure),
    streams,
    start,
    stop,
    stopStream,
  }
}
