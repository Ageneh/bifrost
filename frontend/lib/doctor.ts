/**
 * The doctor: what is wrong, why, and the one thing that fixes it.
 *
 * Home renders this module's output in a fixed order — critical rows, then
 * warnings, then exactly one resolved row — so a healthy bridge reads as one
 * line and a broken one leads with the failure instead of a wall of numbers.
 *
 * Everything here is derived from data the API and the live socket already
 * carry. Nothing is inferred from a placeholder, and a source that does not
 * exist yet in this wave (link button, pairing, streams) contributes no row.
 */
import type { AppConfig } from './generated/AppConfig'
import type { Reason } from './generated/Reason'
import type { Service } from './generated/Service'

export type DoctorSeverity = 'crit' | 'warn' | 'ok'

/** What a row's button does; the page owns the call. */
export type DoctorActionKind
  = | 'restart-service'
    | 'review-draft'
    | 'apply-draft'
    | 'restart-bridge'
    | 'open-bridge'

export interface DoctorAction {
  kind: DoctorActionKind
  label: string
  /** The service a restart applies to. */
  serviceId?: string
}

/** One thing that needs attention, with the cause and the fix behind it. */
export interface AttentionItem {
  id: string
  severity: DoctorSeverity
  /** The cause, in one sentence. */
  title: string
  /** One line under the title, visible without opening the row. */
  why: string
  /** The card: what is actually wrong. */
  cause: string
  /** The card: what to do about it. */
  fix?: string
  action?: DoctorAction
}

/** One segment of the health strip. */
export interface HealthItem {
  id: string
  label: string
  value: string
  state: DoctorSeverity | 'idle'
}

/** A recovery or an apply this session saw happen. */
export interface DoctorEvent {
  name: string
  at: string
}

export interface DoctorInput {
  config: AppConfig | null
  services: Service[]
  /** Draft revision, for the strip. */
  revision: number
  changeCount: number
  draftStatus: string
  restartRequired: readonly Reason[]
  /** The most recent service that came back. */
  recovered: DoctorEvent | null
  /** The last apply this session saw succeed. */
  applied: DoctorEvent | null
  /** Why the active configuration or the service list could not be read. */
  readError: string | null
  /** Why the last draft call failed. */
  draftError: string | null
}

const Z2M_PREFIX = 'z2m@'

function isZ2mService(service: Service): boolean {
  return service.name.startsWith(Z2M_PREFIX)
}

function serverName(service: Service): string {
  return service.name.slice(Z2M_PREFIX.length)
}

function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? '' : 's'}`
}

function stateOf(service: Service): string {
  return String(service.state)
}

function isDown(service: Service): boolean {
  return stateOf(service) === 'Failed'
}

function isNotRunning(service: Service): boolean {
  const state = stateOf(service)
  return state === 'Stopped' || state === 'Registered' || state === 'Configured'
}

/** Rows for everything that is wrong, critical first. */
function attentionRows(input: DoctorInput): AttentionItem[] {
  const critical: AttentionItem[] = []
  const warnings: AttentionItem[] = []

  if (input.readError !== null) {
    critical.push({
      id: 'read',
      severity: 'crit',
      title: 'The bridge state could not be read',
      why: input.readError,
      cause: input.readError,
      fix: 'Check that this page can reach the bridge, then refresh.',
    })
  }

  if (input.draftError !== null) {
    critical.push({
      id: 'draft-error',
      severity: 'crit',
      title: 'The draft could not be saved',
      why: input.draftError,
      cause: input.draftError,
      fix: 'The change is not on the bridge. Check the connection and try again.',
    })
  }

  for (const service of input.services) {
    const state = stateOf(service)
    if (isDown(service)) {
      if (isZ2mService(service)) {
        const server = serverName(service)
        critical.push({
          id: `service-${service.id}`,
          severity: 'crit',
          title: `${server} is not connected`,
          why: `zigbee2mqtt at ${server} stopped with an error.`,
          cause: `The service z2m@${server} is in state Failed, so no light behind it answers.`,
          fix: 'Restart the service. If it fails again, check the URL and token for this server.',
          action: { kind: 'restart-service', label: 'Restart service', serviceId: service.id },
        })
      }
      else {
        critical.push({
          id: `service-${service.id}`,
          severity: 'crit',
          title: `${service.name} failed`,
          why: 'The service manager reported a failure.',
          cause: `The service ${service.name} is in state Failed.`,
          fix: 'Restart the service and watch whether it stays running.',
          action: { kind: 'restart-service', label: 'Restart service', serviceId: service.id },
        })
      }
    }
    else if (isNotRunning(service) && isZ2mService(service)) {
      const server = serverName(service)
      warnings.push({
        id: `service-${service.id}`,
        severity: 'warn',
        title: `${server} is not running`,
        why: `The service is ${state.toLowerCase()}.`,
        cause: `zigbee2mqtt at ${server} is configured but its service is ${state.toLowerCase()}.`,
        fix: 'Start the service from the services screen, or restart the bridge.',
        action: { kind: 'restart-service', label: 'Start service', serviceId: service.id },
      })
    }
  }

  // a configured server with no service at all never connects, and the
  // service list is the only place that can show it in this wave
  const known = new Set(input.services.filter(isZ2mService).map(serverName))
  for (const name of Object.keys(input.config?.z2m ?? {})) {
    if (known.has(name)) {
      continue
    }
    warnings.push({
      id: `server-${name}`,
      severity: 'warn',
      title: `${name} has no running service`,
      why: 'The server is in the configuration but nothing is serving it.',
      cause: `zigbee2mqtt at ${name} is configured, but the bridge runs no service for it.`,
      fix: 'Apply the configuration so the bridge starts this backend.',
      action: { kind: 'apply-draft', label: 'Apply' },
    })
  }

  if (input.changeCount > 0) {
    warnings.push({
      id: 'draft',
      severity: 'warn',
      title: `${plural(input.changeCount, 'change')} waiting to be applied`,
      why: 'The running bridge still uses the previous configuration.',
      cause: 'The draft holds edits that only take effect after they are applied.',
      fix: 'Review the diff, then apply or discard it.',
      action: { kind: 'review-draft', label: 'Review draft' },
    })
  }

  if (input.restartRequired.length > 0) {
    warnings.push({
      id: 'restart',
      severity: 'warn',
      title: 'A restart is required',
      why: plural(input.restartRequired.length, 'setting') + ' take effect only after a restart.',
      cause: input.restartRequired.map(reason => reason.message).join(' '),
      fix: 'Restart Bifrost. The console reconnects on its own.',
      action: { kind: 'restart-bridge', label: 'Restart now' },
    })
  }

  return [...critical, ...warnings]
}

/** The one row that always closes the list. */
function resolvedRow(input: DoctorInput, open: number): AttentionItem {
  if (input.recovered !== null) {
    return {
      id: 'resolved',
      severity: 'ok',
      title: `${input.recovered.name} recovered`,
      why: `Back to running at ${input.recovered.at}.`,
      cause: `${input.recovered.name} was down earlier in this session and is running again.`,
    }
  }
  if (input.applied !== null) {
    return {
      id: 'resolved',
      severity: 'ok',
      title: 'Draft applied',
      why: `The bridge accepted the draft at ${input.applied.at}.`,
      cause: 'The last apply succeeded and the running configuration is current.',
    }
  }
  if (open === 0) {
    return {
      id: 'resolved',
      severity: 'ok',
      title: 'Everything is healthy',
      why: 'No service is down, every server is connected and the draft is empty.',
      cause: 'The bridge reports nothing that needs attention.',
    }
  }
  return {
    id: 'resolved',
    severity: 'ok',
    title: 'Everything else is healthy',
    why: 'Nothing outside the rows above needs attention.',
    cause: 'The rest of the bridge is running normally.',
  }
}

/** The attention list in its fixed order: crit, warn, then one resolved row. */
export function deriveAttention(input: DoctorInput): AttentionItem[] {
  const rows = attentionRows(input)
  return [...rows, resolvedRow(input, rows.length)]
}

/** The health strip: fixed order, one line, no decorative numbers. */
export function deriveHealth(input: DoctorInput): HealthItem[] {
  const total = input.services.length
  const running = input.services.filter(service => stateOf(service) === 'Running').length
  const failed = input.services.filter(isDown).length
  const servers = Object.keys(input.config?.z2m ?? {})
  const connected = input.services.filter(
    service => isZ2mService(service) && stateOf(service) === 'Running',
  ).length

  const servicesState: HealthItem['state']
    = failed > 0 ? 'crit' : running < total ? 'warn' : total === 0 ? 'idle' : 'ok'

  const draftState: HealthItem['state']
    = input.draftStatus === 'conflict'
      ? 'crit'
      : input.changeCount > 0
        ? 'warn'
        : input.draftStatus === 'failed'
          ? 'crit'
          : 'ok'

  return [
    {
      id: 'bridge',
      label: 'Bridge',
      value: input.readError === null ? 'reachable' : 'unreachable',
      state: input.readError === null ? 'ok' : 'crit',
    },
    {
      id: 'services',
      label: 'Services',
      value: `${running} of ${total} running`,
      state: servicesState,
    },
    {
      id: 'z2m',
      label: 'zigbee2mqtt',
      value: servers.length === 0 ? 'none configured' : `${connected} of ${servers.length} connected`,
      state:
        servers.length === 0
          ? 'idle'
          : connected === servers.length
            ? 'ok'
            : failed > 0
              ? 'crit'
              : 'warn',
    },
    {
      id: 'draft',
      label: 'Draft',
      value:
        input.draftStatus === 'conflict'
          ? 'conflict'
          : input.changeCount === 0
            ? 'no changes'
            : plural(input.changeCount, 'change'),
      state: draftState,
    },
    {
      id: 'restart',
      label: 'Restart',
      value: input.restartRequired.length > 0 ? 'required' : 'not required',
      state: input.restartRequired.length > 0 ? 'warn' : 'ok',
    },
    {
      id: 'revision',
      label: 'Revision',
      value: String(input.revision),
      state: 'idle',
    },
  ]
}
