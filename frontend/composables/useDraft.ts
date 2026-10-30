/**
 * The draft: revision tracking, validation, apply and conflict resolution.
 *
 * One instance for the whole shell, because the DraftBar, the DiffDrawer and
 * the DraftPill are three views of the same server-side state. Every write
 * names the revision it was based on; a `409` never throws the user's work
 * away, it opens a per-path keep-mine / take-theirs choice instead.
 */
import { computed, readonly, ref, shallowRef } from 'vue'
import { api as client, ApiError } from '~/lib/api'
import type { ApplyResponse, JsonValue } from '~/lib/generated'
import type { ConfigChange } from '~/lib/generated/ConfigChange'
import type { DraftResponse } from '~/lib/generated/DraftResponse'
import type { Reason } from '~/lib/generated/Reason'
import type { ValidationError } from '~/lib/generated/ValidationError'
import { changedPaths, clone, deletePath, getPath, isObject, setPath } from '~/lib/json'
import type { JsonObject } from '~/lib/json'
import type { ApplyAttempt, ConflictResolution } from '~/lib/types'

export type DraftStatus
  /** nothing loaded yet */
  = | 'loading'
  /** no draft on the server */
    | 'clean'
  /** a draft differs from the active configuration */
    | 'dirty'
  /** another browser moved the revision; the user picks per path */
    | 'conflict'
  /** an apply is in flight */
    | 'applying'
  /** the last apply succeeded */
    | 'applied'
  /** the last apply failed */
    | 'failed'

/** One path the two sides disagree about. */
export interface ConflictPath {
  path: string
  mine: unknown
  theirs: unknown
}

export interface DraftConflict {
  /** The draft as the other side left it. */
  theirs: DraftResponse
  /** The document this browser tried to write. */
  local: JsonObject
  paths: ConflictPath[]
}

const revision = ref(0)
// shallow: the document is recursive and always replaced whole, so deep
// reactivity would only cost the compiler an unbounded expansion
const draft = shallowRef<DraftResponse | null>(null)
const changes = ref<ConfigChange[]>([])
const validation = ref<ValidationError[]>([])
const restartRequired = ref<Reason[]>([])
const status = ref<DraftStatus>('loading')
const conflict = shallowRef<DraftConflict | null>(null)
const applyAttempt = ref<ApplyAttempt | null>(null)
const applyResult = ref<ApplyResponse | null>(null)
const error = ref<string | null>(null)

/** The draft document, or the active one when the server holds no draft. */
const document = computed(() => {
  // typed as `unknown` first: `JsonValue` is recursive and deeply readonly
  // through the ref, which the compiler cannot expand
  const config: unknown = draft.value?.config
  return isObject(config) ? (config as unknown as JsonObject) : null
})

const changeCount = computed(() => changes.value.length)
const hasDraft = computed(() => draft.value?.config !== null && draft.value?.config !== undefined)
const blocking = computed(() => validation.value.filter(finding => finding.severity === 'error'))

/** Keep a failed call from leaving the shell spinning. */
function recordFailure(cause: unknown): ApiError | null {
  if (cause instanceof ApiError) {
    error.value = cause.message
    return cause
  }
  error.value = cause instanceof Error ? cause.message : String(cause)
  return null
}

/** Read the draft and its diff; the revision is the one the server reports. */
async function load(): Promise<void> {
  status.value = 'loading'
  try {
    const [response, diff] = await Promise.all([client.draft(), client.diff()])
    draft.value = response
    revision.value = response.revision
    changes.value = diff
    validation.value = response.validation
    status.value = diff.length > 0 ? 'dirty' : 'clean'
    error.value = null
  }
  catch (cause) {
    recordFailure(cause)
    status.value = 'failed'
  }
}

/**
 * Replace the draft with `config`, based on the current revision.
 *
 * A `409` means someone else wrote first: their draft and ours are compared
 * path by path and left for `resolveConflicts`.
 */
async function save(config: JsonObject): Promise<boolean> {
  try {
    const response = await client.putDraft({
      revision: revision.value,
      config: config as unknown as JsonValue,
    })
    adopt(response)
    return true
  }
  catch (cause) {
    const failure = recordFailure(cause)
    if (failure?.isStaleRevision) {
      await openConflict(config)
    }
    else {
      status.value = 'failed'
    }
    return false
  }
}

/** Change only the named paths, leaving the rest of the draft alone. */
async function patch(patchDocument: JsonObject): Promise<boolean> {
  try {
    const response = await client.patchDraft({
      revision: revision.value,
      patch: patchDocument as unknown as JsonValue,
    })
    adopt(response)
    return true
  }
  catch (cause) {
    const failure = recordFailure(cause)
    if (failure?.isStaleRevision) {
      const theirs = await client.draft()
      await openConflict(mergeOnto(document.value ?? {}, patchDocument), theirs)
    }
    else {
      status.value = 'failed'
    }
    return false
  }
}

/** Resolve a conflict: keep our value or accept theirs, path by path. */
async function resolveConflicts(resolutions: ConflictResolution[]): Promise<boolean> {
  const current = conflict.value
  if (current === null) {
    return false
  }

  const remote: unknown = current.theirs.config
  const merged = clone(isObject(remote) ? (remote as JsonObject) : {})
  for (const resolution of resolutions) {
    if (resolution.choice === 'keep') {
      const value = getPath(current.local, resolution.path)
      if (value === undefined) {
        deletePath(merged, resolution.path)
      }
      else {
        setPath(merged, resolution.path, value)
      }
    }
  }

  try {
    const response = await client.putDraft({
      revision: current.theirs.revision,
      config: merged as unknown as JsonValue,
    })
    conflict.value = null
    adopt(response)
    return true
  }
  catch (cause) {
    const failure = recordFailure(cause)
    if (failure?.isStaleRevision) {
      // a third writer arrived; compare against the newest draft
      const theirs = await client.draft()
      await openConflict(merged, theirs)
    }
    else {
      status.value = 'failed'
    }
    return false
  }
}

/** Drop the draft on the server. */
async function discard(): Promise<boolean> {
  try {
    const response = await client.discardDraft({ revision: revision.value })
    revision.value = response.revision
    draft.value = null
    changes.value = []
    validation.value = []
    restartRequired.value = []
    conflict.value = null
    applyResult.value = null
    status.value = 'clean'
    return true
  }
  catch (cause) {
    recordFailure(cause)
    return false
  }
}

/** Validate the draft as it stands, without writing anything. */
async function validate(): Promise<ValidationError[]> {
  try {
    const findings = document.value
      ? await client.validateDocument(document.value)
      : await client.validateActive()
    validation.value = findings
    return findings
  }
  catch (cause) {
    recordFailure(cause)
    return []
  }
}

/**
 * Apply the draft.
 *
 * The attempt carries a client-generated `apply_id`; it outlives the request
 * so a socket drop mid-apply can be reconciled by `reconcileAfterReconnect`.
 */
async function apply(): Promise<boolean> {
  if (!hasDraft.value) {
    return false
  }
  const attempt: ApplyAttempt = {
    applyId: crypto.randomUUID(),
    revision: revision.value,
    startedAt: new Date().toISOString(),
  }
  applyAttempt.value = attempt
  status.value = 'applying'
  error.value = null

  try {
    const response = await client.apply({ revision: attempt.revision })
    adoptApplied(response)
    return true
  }
  catch (cause) {
    const failure = recordFailure(cause)
    if (failure?.isStaleRevision) {
      status.value = 'conflict'
      const theirs = await client.draft()
      await openConflict(document.value ?? {}, theirs)
    }
    else {
      status.value = 'failed'
    }
    applyAttempt.value = null
    return false
  }
}

/** Ask the supervisor to restart Bifrost, applying a draft first if one exists. */
async function restart(): Promise<boolean> {
  try {
    const applied = await client.restart({ revision: hasDraft.value ? revision.value : null })
    if (applied) {
      adoptApplied(applied)
    }
    return true
  }
  catch (cause) {
    recordFailure(cause)
    return false
  }
}

/**
 * Settle an apply that was in flight when the socket dropped.
 *
 * The server consumes the draft when it applies it, so a draft that is gone
 * and a revision beyond the one we applied means the apply landed. A draft
 * that is still there means it never ran, and the user can apply again.
 */
async function reconcileAfterReconnect(): Promise<void> {
  const attempt = applyAttempt.value
  if (attempt === null) {
    return
  }
  try {
    const response = await client.draft()
    const config: unknown = response.config
    const consumed = config === null || config === undefined
    if (consumed && response.revision > attempt.revision) {
      // the server applied it; there is no plan to report after the fact
      markApplied(response.revision)
      return
    }
    await load()
  }
  catch (cause) {
    recordFailure(cause)
  }
}

function adopt(response: DraftResponse): void {
  draft.value = response
  revision.value = response.revision
  validation.value = response.validation
  error.value = null
  applyAttempt.value = null
  void refreshDiff()
}

function adoptApplied(response: ApplyResponse): void {
  applyResult.value = response
  restartRequired.value = response.plan.restart_required
  changes.value = response.changes
  markApplied(response.revision)
}

/** Settle the draft state after an apply, with or without a response. */
function markApplied(nextRevision: number): void {
  draft.value = null
  revision.value = nextRevision
  validation.value = []
  applyAttempt.value = null
  status.value = 'applied'
  void refreshDiff()
}

async function refreshDiff(): Promise<void> {
  try {
    changes.value = await client.diff()
    if (status.value !== 'applied' && status.value !== 'applying') {
      status.value = changes.value.length > 0 ? 'dirty' : 'clean'
    }
  }
  catch {
    // the diff is a view; a failure here must not mask the write result
  }
}

async function openConflict(local: JsonObject, theirs?: DraftResponse): Promise<void> {
  const remote = theirs ?? (await client.draft())
  const theirsDocument = isObject(remote.config) ? remote.config : {}
  const paths = changedPaths(local, theirsDocument).map(path => ({
    path,
    mine: getPath(local, path),
    theirs: getPath(theirsDocument, path),
  }))
  conflict.value = { theirs: remote, local, paths }
  status.value = 'conflict'
}

/** The document `patch` would produce on top of `base`. */
function mergeOnto(base: JsonObject, patchDocument: JsonObject): JsonObject {
  const merged = clone(base)
  for (const path of changedPaths({}, patchDocument)) {
    const value = getPath(patchDocument, path)
    if (value === null) {
      deletePath(merged, path)
    }
    else {
      setPath(merged, path, value)
    }
  }
  return merged
}

export function useDraft() {
  return {
    revision: readonly(revision),
    draft: readonly(draft),
    document,
    changes: readonly(changes),
    validation: readonly(validation),
    blocking,
    restartRequired: readonly(restartRequired),
    status: readonly(status),
    conflict: readonly(conflict),
    applyAttempt: readonly(applyAttempt),
    applyResult: readonly(applyResult),
    error: readonly(error),
    changeCount,
    hasDraft,
    load,
    save,
    patch,
    resolveConflicts,
    discard,
    validate,
    apply,
    restart,
    reconcileAfterReconnect,
  }
}
