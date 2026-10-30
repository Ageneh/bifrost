/**
 * The one API client and the session state the shell gates on.
 *
 * The shell is served unauthenticated and decides from `GET
 * /bifrost/auth/session` whether to show the console, the sign-in panel or
 * the first-run setup panel.
 */
import { computed, readonly, ref } from 'vue'
import { api as client, ApiError } from '~/lib/api'
import type { SessionResponse } from '~/lib/generated'

const session = ref<SessionResponse | null>(null)
const loading = ref(false)
const failure = ref<ApiError | null>(null)

export function useApi() {
  /** True once a session response has been read, whatever it said. */
  const ready = computed(() => session.value !== null)
  const authenticated = computed(() => session.value?.authenticated === true)
  const setupRequired = computed(() => session.value?.setup_required === true)
  const ingress = computed(() => session.value?.ingress === true)

  /** Read the session; safe to call again after any 401. */
  async function refresh(): Promise<void> {
    loading.value = true
    try {
      session.value = await client.session()
      client.useSession(session.value)
      failure.value = null
    }
    catch (error) {
      failure.value = error instanceof ApiError ? error : null
      session.value = null
    }
    finally {
      loading.value = false
    }
  }

  async function login(password: string): Promise<void> {
    session.value = await client.login({ password })
    client.useSession(session.value)
  }

  async function setup(password: string, setupCode?: string): Promise<void> {
    session.value = await client.setup({
      password,
      setup_code: setupCode === undefined || setupCode === '' ? null : setupCode,
    })
    client.useSession(session.value)
  }

  async function logout(all = false): Promise<void> {
    try {
      await client.logout({ all })
    }
    finally {
      client.clearSession()
      session.value = null
    }
  }

  /** Remember that the server refused the session, without a round trip. */
  function expire(): void {
    client.clearSession()
    session.value = null
  }

  return {
    client,
    session: readonly(session),
    loading: readonly(loading),
    failure: readonly(failure),
    ready,
    authenticated,
    setupRequired,
    ingress,
    refresh,
    login,
    setup,
    logout,
    expire,
  }
}
