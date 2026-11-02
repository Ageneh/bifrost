/**
 * The typed client for `/bifrost`.
 *
 * Every call goes through `request`, which turns a non-2xx response into an
 * `ApiError` carrying the server's message and validation findings. Mutations
 * carry the CSRF token from the session; the session cookie is HttpOnly and
 * travels on its own.
 */
import type { ApplyResponse } from './generated/ApplyResponse'
import type { CertificateInfo } from './generated/CertificateInfo'
import type { ConfigChange } from './generated/ConfigChange'
import type { ConfigImport } from './generated/ConfigImport'
import type { DeviceInfo } from './generated/DeviceInfo'
import type { DraftPatch } from './generated/DraftPatch'
import type { DraftResponse } from './generated/DraftResponse'
import type { DraftWrite } from './generated/DraftWrite'
import type { EntertainmentCapabilities } from './generated/EntertainmentCapabilities'
import type { EntertainmentStatus } from './generated/EntertainmentStatus'
import type { EntertainmentZone } from './generated/EntertainmentZone'
import type { IdentityBundle } from './generated/IdentityBundle'
import type { IdentityMacChange } from './generated/IdentityMacChange'
import type { IdentityMacChangeReply } from './generated/IdentityMacChangeReply'
import type { IdentityRestore } from './generated/IdentityRestore'
import type { IdentityRestoreReply } from './generated/IdentityRestoreReply'
import type { InventorySnapshot } from './generated/InventorySnapshot'
import type { LinkButtonStatus } from './generated/LinkButtonStatus'
import type { PairingHistory } from './generated/PairingHistory'
import type { StateSummary } from './generated/StateSummary'
import type { LoginRequest } from './generated/LoginRequest'
import type { LogoutRequest } from './generated/LogoutRequest'
import type { PasswordChange } from './generated/PasswordChange'
import type { PermitJoinRequest } from './generated/PermitJoinRequest'
import type { PermitJoinResponse } from './generated/PermitJoinResponse'
import type { RestartRequest } from './generated/RestartRequest'
import type { DiscoveryRequest } from './generated/DiscoveryRequest'
import type { DiscoveryResult } from './generated/DiscoveryResult'
import type { RevisionRequest } from './generated/RevisionRequest'
import type { RoomList } from './generated/RoomList'
import type { ServiceList } from './generated/ServiceList'
import type { ServiceState } from './generated/ServiceState'
import type { SessionResponse } from './generated/SessionResponse'
import type { SetupRequest } from './generated/SetupRequest'
import type { WizardProgress } from './generated/WizardProgress'
import type { WizardWrite } from './generated/WizardWrite'
import type { ValidationError } from './generated/ValidationError'

/** Header every mutating request with a session must carry. */
export const CSRF_HEADER = 'x-bifrost-csrf'

/** Prefix every admin endpoint sits under, relative to the document base. */
const ADMIN_PREFIX = 'bifrost'

/** The server's error body: `{ error, errors }`. */
export class ApiError extends Error {
  readonly status: number
  readonly validation: ValidationError[]

  constructor(status: number, message: string, validation: ValidationError[] = []) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.validation = validation
  }

  /** The draft moved on: the caller must reload and reconcile. */
  get isStaleRevision(): boolean {
    return this.status === 409
  }

  /** No session, or it expired. */
  get isUnauthenticated(): boolean {
    return this.status === 401
  }

  /** A session, but no matching CSRF token. */
  get isForbidden(): boolean {
    return this.status === 403
  }
}

/**
 * The base URL the page is served under, always ending in `/`.
 *
 * The server injects `window.__BIFROST_BASE__` from the ingress path; a
 * static build opened straight from disk falls back to the document.
 */
export function baseUrl(): string {
  const injected = (globalThis as { __BIFROST_BASE__?: unknown }).__BIFROST_BASE__
  if (typeof injected === 'string' && injected.endsWith('/')) {
    return injected
  }
  return './'
}

/** Absolute URL of an admin path, e.g. `config/draft`. */
export function adminUrl(path: string): string {
  return `${baseUrl()}${ADMIN_PREFIX}/${path.replace(/^\/+/, '')}`
}

/**
 * Absolute URL of a CLIP v2 path, e.g. `resource/light`.
 *
 * The editor writes zones through this surface, the same one the Hue app
 * uses: a zone saved here and a zone saved by the app are the same zone.
 */
export function clipUrl(path: string): string {
  return `${baseUrl()}clip/v2/${path.replace(/^\/+/, '')}`
}

/** WebSocket URL of the admin socket, matching the page's scheme. */
export function websocketUrl(): string {
  const base = new URL(baseUrl(), globalThis.location?.href ?? 'http://localhost/')
  const scheme = base.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${base.host}${base.pathname}${ADMIN_PREFIX}/ws`
}

interface RequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  body?: unknown
  /** Send the CSRF header; every mutation does. */
  csrf?: boolean
  /** Return the response as text (the YAML export). */
  text?: boolean
}

export class ApiClient {
  private csrfToken: string | null = null

  /** Remember the token a session response carries. */
  useSession(session: SessionResponse): void {
    this.csrfToken = session.csrf
  }

  clearSession(): void {
    this.csrfToken = null
  }

  private async request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const { method = 'GET', body, csrf = false, text = false } = options
    const headers: Record<string, string> = { accept: 'application/json' }
    if (body !== undefined) {
      headers['content-type'] = 'application/json'
    }
    if (csrf && this.csrfToken) {
      headers[CSRF_HEADER] = this.csrfToken
    }

    let response: Response
    try {
      response = await fetch(adminUrl(path), {
        method,
        headers,
        credentials: 'same-origin',
        body: body === undefined ? undefined : JSON.stringify(body),
      })
    }
    catch (cause) {
      throw new ApiError(0, cause instanceof Error ? cause.message : 'Network request failed')
    }

    if (!response.ok) {
      throw await toApiError(response)
    }

    if (text) {
      return (await response.text()) as T
    }
    if (response.status === 204) {
      return undefined as T
    }
    return (await response.json()) as T
  }

  // ---- session ---------------------------------------------------------

  session(): Promise<SessionResponse> {
    return this.request('auth/session')
  }

  login(body: LoginRequest): Promise<SessionResponse> {
    return this.request('auth/login', { method: 'POST', body })
  }

  setup(body: SetupRequest): Promise<SessionResponse> {
    return this.request('auth/setup', { method: 'POST', body })
  }

  logout(body: LogoutRequest = { all: false }): Promise<void> {
    return this.request('auth/logout', { method: 'POST', body, csrf: true })
  }

  changePassword(body: PasswordChange): Promise<void> {
    return this.request('auth/password', { method: 'POST', body, csrf: true })
  }

  // ---- configuration ---------------------------------------------------

  /** The active configuration, redacted. */
  activeConfig(): Promise<Record<string, unknown>> {
    return this.request('config')
  }

  draft(): Promise<DraftResponse> {
    return this.request('config/draft')
  }

  putDraft(body: DraftWrite): Promise<DraftResponse> {
    return this.request('config/draft', { method: 'PUT', body, csrf: true })
  }

  patchDraft(body: DraftPatch): Promise<DraftResponse> {
    return this.request('config/draft', { method: 'PATCH', body, csrf: true })
  }

  discardDraft(body: RevisionRequest): Promise<{ revision: number }> {
    return this.request('config/draft', { method: 'DELETE', body, csrf: true })
  }

  diff(): Promise<ConfigChange[]> {
    return this.request('config/diff')
  }

  validateActive(): Promise<ValidationError[]> {
    return this.request('config/validate')
  }

  validateDocument(document: unknown): Promise<ValidationError[]> {
    return this.request('config/validate', { method: 'POST', body: document, csrf: true })
  }

  apply(body: RevisionRequest): Promise<ApplyResponse> {
    return this.request('config/apply', { method: 'POST', body, csrf: true })
  }

  restart(body: RestartRequest): Promise<ApplyResponse | null> {
    return this.request('config/restart', { method: 'POST', body, csrf: true })
  }

  exportConfig(): Promise<string> {
    return this.request('config/export', { text: true })
  }

  importConfig(body: ConfigImport): Promise<DraftResponse> {
    return this.request('config/import', { method: 'POST', body, csrf: true })
  }

  // ---- devices ---------------------------------------------------------

  /** Every device the bridge knows, optionally narrowed by the server. */
  devices(query: DeviceQuery = {}): Promise<InventorySnapshot> {
    return this.request(`devices${queryString({ ...query })}`)
  }

  device(ieee: string, server?: string): Promise<DeviceInfo> {
    return this.request(`devices/${encodeURIComponent(ieee)}${queryString({ server })}`)
  }

  /** Flash one device so a person can tell which bulb it is. */
  identifyDevice(ieee: string, server: string, seconds = 5): Promise<DeviceInfo> {
    return this.request(
      `devices/${encodeURIComponent(ieee)}/identify${queryString({ server })}`,
      { method: 'POST', body: { seconds }, csrf: true },
    )
  }

  /** Open a server's join window. */
  permitJoin(server: string, body: Partial<PermitJoinRequest> = {}): Promise<PermitJoinResponse> {
    return this.request(`z2m/${encodeURIComponent(server)}/permit_join`, {
      method: 'POST',
      body,
      csrf: true,
    })
  }

  /** Seconds left in a server's join window; zero when none is open. */
  permitJoinStatus(server: string): Promise<PermitJoinResponse> {
    return this.request(`z2m/${encodeURIComponent(server)}/permit_join`)
  }

  // ---- first run -------------------------------------------------------

  /** Where the first-run wizard left off. */
  wizard(): Promise<WizardProgress> {
    return this.request('setup/wizard')
  }

  saveWizard(body: WizardWrite): Promise<WizardProgress> {
    return this.request('setup/wizard', { method: 'PUT', body, csrf: true })
  }

  /** Ask a z2m server what it can see, before there is a bridge to ask. */
  discovery(body: DiscoveryRequest): Promise<DiscoveryResult> {
    return this.request('setup/discovery', { method: 'POST', body, csrf: true })
  }

  // ---- rooms -----------------------------------------------------------

  /** Every room, with the devices in it and the scenes it holds. */
  rooms(): Promise<RoomList> {
    return this.request('rooms')
  }

  /** Recall one of a room's scenes. */
  recallScene(room: string, scene: string, duration?: number): Promise<void> {
    return this.request(
      `rooms/${encodeURIComponent(room)}/scenes/${encodeURIComponent(scene)}/recall`,
      { method: 'POST', body: { duration: duration ?? null }, csrf: true },
    )
  }

  // ---- pairing ---------------------------------------------------------

  /** The pairing window as it stands right now. */
  linkButton(): Promise<LinkButtonStatus> {
    return this.request('bridge/link-button')
  }

  /** Open a pairing window; the server defaults to 30 seconds. */
  pressLinkButton(seconds?: number): Promise<LinkButtonStatus> {
    return this.request('bridge/link-button', {
      method: 'POST',
      body: seconds === undefined ? {} : { seconds },
      csrf: true,
    })
  }

  /** The pairing mode and the attempts the bridge remembers. */
  pairing(): Promise<PairingHistory> {
    return this.request('bridge/pairing')
  }

  // ---- entertainment ---------------------------------------------------

  /** What this bridge can stream on, and the zones that exist. */
  entertainmentCapabilities(): Promise<EntertainmentCapabilities> {
    return this.request('entertainment/capabilities')
  }

  /**
   * What every stream is doing.
   *
   * Meant to be polled about once a second: the rates in the answer are
   * measured over that second, and the socket only says when the set of
   * streams changed.
   */
  entertainmentStatus(): Promise<EntertainmentStatus> {
    return this.request('entertainment/status')
  }

  /**
   * End the stream on one area.
   *
   * There is no start: a stream begins when the TV or the Sync app sends its
   * first frame, and a button here could only announce frames no client will
   * send.
   */
  stopStream(area: string): Promise<void> {
    return this.request(`entertainment/${encodeURIComponent(area)}/stop`, {
      method: 'POST',
      body: {},
      csrf: true,
    })
  }

  /** One zone, with its positions and the channels derived from them. */
  entertainmentZone(id: string): Promise<EntertainmentZone> {
    return this.request(`entertainment/configurations/${encodeURIComponent(id)}`)
  }

  /**
   * Save a zone's positions and members.
   *
   * The write goes through the Hue v2 `PUT`, which is the surface the Hue app
   * uses, so the editor and the app cannot disagree about what a zone is.
   */
  async putZone(id: string, body: unknown): Promise<void> {
    await this.clip(`resource/entertainment_configuration/${encodeURIComponent(id)}`, 'PUT', body)
  }

  /** Create a zone, and return its id. */
  async createZone(body: unknown): Promise<string> {
    const reply = await this.clip<{ data?: Array<{ rid?: string }> }>(
      'resource/entertainment_configuration',
      'POST',
      body,
    )
    const id = reply.data?.[0]?.rid
    if (id === undefined) {
      throw new ApiError(0, 'The zone was created but the answer carried no id.')
    }
    return id
  }

  /**
   * One call on the CLIP v2 surface.
   *
   * It is a different error shape from `/bifrost`: a v2 failure carries
   * `errors[].description`, which is what the Hue app shows, so it is what
   * the editor shows too.
   */
  private async clip<T>(
    path: string,
    method: 'GET' | 'POST' | 'PUT' = 'GET',
    body?: unknown,
  ): Promise<T> {
    const headers: Record<string, string> = { accept: 'application/json' }
    if (body !== undefined) {
      headers['content-type'] = 'application/json'
    }

    let response: Response
    try {
      response = await fetch(clipUrl(path), {
        method,
        headers,
        credentials: 'same-origin',
        body: body === undefined ? undefined : JSON.stringify(body),
      })
    }
    catch (cause) {
      throw new ApiError(0, cause instanceof Error ? cause.message : 'Network request failed')
    }

    if (!response.ok) {
      const fallback = `${response.status} ${response.statusText}`.trim()
      const parsed = await response.json().catch(() => null) as
        | { errors?: Array<{ description?: string }> }
        | null
      const description = parsed?.errors?.[0]?.description
      throw new ApiError(response.status, description ?? fallback)
    }

    return (await response.json()) as T
  }

  // ---- identity --------------------------------------------------------

  /** What the bridge's own certificate says. */
  certificate(): Promise<CertificateInfo> {
    return this.request('bridge/certificate')
  }

  /** Replace the certificate. Every paired client must pair again. */
  regenerateCertificate(): Promise<CertificateInfo> {
    return this.request('bridge/certificate/regenerate', { method: 'POST', body: {}, csrf: true })
  }

  /** Queue a MAC change; it takes effect at the next start. */
  changeMac(body: IdentityMacChange): Promise<IdentityMacChangeReply> {
    return this.request('bridge/identity/mac', { method: 'POST', body, csrf: true })
  }

  /**
   * The identity bundle: certificate and pairing list.
   *
   * The server only serves it over HTTPS, so this call fails on the plain
   * listener with the server's own message.
   */
  identityBackup(): Promise<IdentityBundle> {
    return this.request('bridge/identity/backup')
  }

  /** Restore an identity bundle, authorised by the admin password. */
  restoreIdentity(body: IdentityRestore): Promise<IdentityRestoreReply> {
    return this.request('bridge/identity/restore', { method: 'POST', body, csrf: true })
  }

  /** What the bridge's state holds, and which bridge this is. */
  stateSummary(): Promise<StateSummary> {
    return this.request('state/summary')
  }

  /** The state as YAML, for the backup the reset offers. */
  stateExport(): Promise<string> {
    return this.request('state/export', { text: true })
  }

  /** Empty the state, after the user typed the bridge id. */
  resetState(confirm: string): Promise<StateSummary> {
    return this.request('state/reset', { method: 'POST', body: { confirm }, csrf: true })
  }

  // ---- services --------------------------------------------------------

  services(): Promise<ServiceList> {
    return this.request('service')
  }

  setServiceState(id: string, state: ServiceState): Promise<string> {
    return this.request(`service/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: state,
      csrf: true,
    })
  }
}

/** What the device list may be narrowed by. */
export interface DeviceQuery {
  server?: string
  type?: string
  supported?: boolean
}

/** `?a=1&b=2`, with the empty parts left out. */
function queryString(query: Record<string, string | boolean | undefined>): string {
  const parts = Object.entries(query)
    .filter(([, value]) => value !== undefined && value !== '')
    .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`)
  return parts.length === 0 ? '' : `?${parts.join('&')}`
}

/** The client every composable shares, so the CSRF token is one value. */
export const api = new ApiClient()

/** Turn a failed response into an `ApiError`, keeping the server's findings. */
async function toApiError(response: Response): Promise<ApiError> {
  const fallback = `${response.status} ${response.statusText}`.trim()
  try {
    const body = (await response.json()) as { error?: string, errors?: ValidationError[] }
    return new ApiError(response.status, body.error ?? fallback, body.errors ?? [])
  }
  catch {
    return new ApiError(response.status, fallback)
  }
}
