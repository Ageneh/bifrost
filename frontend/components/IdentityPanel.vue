<script setup lang="ts">
/**
 * Who the bridge is: its certificate, its MAC address, and its state.
 *
 * Each of the three actions below invalidates every paired client, so each
 * one opens a confirmation that names the cost, offers a backup first, and
 * asks for the bridge id typed out. The backup is offered and never taken:
 * nothing downloads unless the user asks for it.
 */
import { computed, onMounted, ref } from 'vue'
import DestructiveConfirm from '~/components/DestructiveConfirm.vue'
import { useApi } from '~/composables/useApi'
import { useDraft } from '~/composables/useDraft'
import { useToast } from '~/composables/useToast'
import type { CertificateInfo } from '~/lib/generated/CertificateInfo'
import type { StateSummary } from '~/lib/generated/StateSummary'

const { client } = useApi()
const draft = useDraft()
const toast = useToast()

const certificate = ref<CertificateInfo | null>(null)
const stateSummary = ref<StateSummary | null>(null)
const loading = ref(true)
const failure = ref<string | null>(null)

const bridgeId = computed(() => certificate.value?.bridge_id ?? stateSummary.value?.bridge_id ?? '')

const regenerateOpen = ref(false)
const macOpen = ref(false)
const resetOpen = ref(false)
const busy = ref(false)
const backupBusy = ref(false)
const newMac = ref('')

const restoreOpen = ref(false)
const restoreBundle = ref<unknown>(null)
const restorePassword = ref('')
const restoreToken = ref('')

/** The bridge id is the word every confirmation asks for. */
const confirmWord = computed(() => bridgeId.value)

const impactsRegenerate = [
  'The bridge issues a new certificate and a new private key.',
  'Every paired client trusts the old certificate and must pair again.',
  'The HTTPS listener restarts with the new certificate.',
]

const impactsMac = computed(() => [
  `The bridge id changes from ${bridgeId.value} to the new address.`,
  'The certificate is reissued for the new bridge id when the change is applied.',
  'The state\'s bridge resources are rebuilt around the new id at the next start.',
  'Every paired client must pair again.',
  'This is a restart-required change; the apply plan names the restart.',
])

const impactsReset = [
  'Every room, scene and imported device is removed from the bridge.',
  'The z2m backends re-import their devices from scratch.',
  'The configuration is untouched: settings, servers and tokens stay.',
]

async function load(): Promise<void> {
  loading.value = true
  try {
    const [cert, summary] = await Promise.all([client.certificate(), client.stateSummary()])
    certificate.value = cert
    stateSummary.value = summary
    failure.value = null
  }
  catch (cause) {
    failure.value = cause instanceof Error ? cause.message : 'Could not read the bridge identity.'
  }
  finally {
    loading.value = false
  }
}

onMounted(load)

/** Hand `text` to the browser as a file. Only ever called from a click. */
function saveFile(filename: string, text: string, type: string): void {
  const blob = new Blob([text], { type })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  URL.revokeObjectURL(url)
}

async function downloadIdentity(): Promise<void> {
  backupBusy.value = true
  try {
    const bundle = await client.identityBackup()
    saveFile(
      `bifrost-identity-${bundle.bridge_id}.json`,
      JSON.stringify(bundle, null, 2),
      'application/json',
    )
    toast.ok('Identity bundle saved. It holds the private key; keep it somewhere private.')
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not prepare the identity bundle.')
  }
  finally {
    backupBusy.value = false
  }
}

async function downloadState(): Promise<void> {
  backupBusy.value = true
  try {
    const yaml = await client.stateExport()
    saveFile(`bifrost-state-${bridgeId.value}.yaml`, yaml, 'application/yaml')
    toast.ok('State saved.')
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not prepare the state export.')
  }
  finally {
    backupBusy.value = false
  }
}

async function regenerate(): Promise<void> {
  busy.value = true
  try {
    certificate.value = await client.regenerateCertificate()
    regenerateOpen.value = false
    toast.ok('Certificate regenerated. Every paired client must pair again.')
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not regenerate the certificate.')
  }
  finally {
    busy.value = false
  }
}

async function changeMac(): Promise<void> {
  busy.value = true
  try {
    const reply = await client.changeMac({ mac: newMac.value, confirm: bridgeId.value })
    macOpen.value = false
    newMac.value = ''
    await draft.load()
    toast.ok(
      `MAC queued as ${reply.mac}. Apply the draft to schedule the restart that rebuilds the identity.`,
    )
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not queue the MAC change.')
  }
  finally {
    busy.value = false
  }
}

/** Read the bundle the user picked. It is not uploaded until they confirm. */
async function pickBundle(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) {
    restoreBundle.value = null
    return
  }
  try {
    restoreBundle.value = JSON.parse(await file.text())
  }
  catch {
    restoreBundle.value = null
    toast.fail('That file is not an identity bundle.')
  }
}

async function restore(): Promise<void> {
  if (restoreBundle.value === null) {
    toast.fail('Choose an identity bundle first.')
    return
  }
  busy.value = true
  try {
    const reply = await client.restoreIdentity({
      // the generated type is the wire shape; the file is exactly that shape
      bundle: restoreBundle.value as never,
      admin_password: restorePassword.value,
      z2m_token: restoreToken.value,
    })
    restoreOpen.value = false
    restorePassword.value = ''
    restoreToken.value = ''
    await load()
    const tokens = reply.tokens_written.length
      ? ` The z2m token went into ${reply.tokens_written.join(', ')}.`
      : ' Every z2m server already had a token, so none was written.'
    toast.ok(`Identity restored for ${reply.bridge_id}.${tokens} Every client must pair again.`)
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not restore the identity.')
  }
  finally {
    busy.value = false
  }
}

async function resetState(): Promise<void> {
  busy.value = true
  try {
    stateSummary.value = await client.resetState(bridgeId.value)
    resetOpen.value = false
    toast.ok('State reinitialized. The z2m backends are re-importing.')
  }
  catch (cause) {
    toast.fail(cause instanceof Error ? cause.message : 'Could not reset the state.')
  }
  finally {
    busy.value = false
  }
}
</script>

<template>
  <section
    class="fd-panel"
    data-testid="identity-panel"
  >
    <div class="fd-panel__head">
      <div>
        <div class="fd-panel__title">
          Identity
        </div>
        <div class="fd-panel__sub">
          The certificate, the MAC address and the state are what make this bridge this bridge.
          Each change below makes every paired client pair again.
        </div>
      </div>
    </div>

    <p
      v-if="failure"
      class="fd-alert"
      data-testid="identity-failure"
      role="alert"
    >
      {{ failure }}
    </p>

    <div
      v-else-if="loading"
      class="fd-muted"
      data-testid="identity-loading"
    >
      Reading the identity…
    </div>

    <template v-else>
      <div
        class="fd-readouts"
        data-testid="identity-readouts"
      >
        <div class="fd-readout">
          <div class="fd-label">
            Bridge id
          </div>
          <div
            class="mono fd-break"
            data-testid="identity-bridge-id"
          >
            {{ bridgeId }}
          </div>
        </div>
        <div class="fd-readout">
          <div class="fd-label">
            MAC
          </div>
          <div
            class="mono fd-break"
            data-testid="identity-mac"
          >
            {{ certificate?.mac ?? stateSummary?.mac }}
          </div>
        </div>
        <div class="fd-readout">
          <div class="fd-label">
            Certificate
          </div>
          <div
            class="mono fd-break"
            data-testid="identity-fingerprint"
          >
            {{ certificate?.fingerprint ?? 'unknown' }}
          </div>
        </div>
        <div class="fd-readout">
          <div class="fd-label">
            Valid until
          </div>
          <div data-testid="identity-valid-until">
            {{ certificate?.not_after ?? 'unknown' }}
          </div>
        </div>
      </div>

      <div
        class="fd-stack"
        style="margin-top: var(--fd-4)"
      >
        <div class="fd-between">
          <div>
            <div class="fd-panel__title">
              Certificate
            </div>
            <div class="fd-muted">
              Reissue the certificate and its private key for the current MAC address.
            </div>
          </div>
          <button
            type="button"
            class="fd-btn fd-btn--danger"
            data-testid="identity-regenerate-open"
            @click="regenerateOpen = true"
          >
            Regenerate
          </button>
        </div>

        <div class="fd-between">
          <div>
            <div class="fd-panel__title">
              MAC address
            </div>
            <div class="fd-muted">
              Change the address the bridge id and the certificate derive from. Takes effect at
              the next start.
            </div>
          </div>
          <button
            type="button"
            class="fd-btn fd-btn--danger"
            data-testid="identity-mac-open"
            @click="macOpen = true"
          >
            Change MAC
          </button>
        </div>

        <div class="fd-between">
          <div>
            <div class="fd-panel__title">
              State
            </div>
            <div class="fd-muted">
              Empty the rooms, scenes and imported devices the bridge holds.
            </div>
          </div>
          <button
            type="button"
            class="fd-btn fd-btn--danger"
            data-testid="identity-reset-open"
            @click="resetOpen = true"
          >
            Reinitialize
          </button>
        </div>
      </div>
    </template>

    <DestructiveConfirm
      :open="regenerateOpen"
      title="Regenerate the certificate?"
      message="The bridge issues a new certificate and private key. Every client that trusts the old one stops working until it pairs again."
      confirm-label="Regenerate certificate"
      :require-text="confirmWord"
      :impacts="impactsRegenerate"
      backup-label="Download identity bundle first"
      :backup-busy="backupBusy"
      backup-note="Offered, not taken automatically. It holds the private key."
      :busy="busy"
      testid="identity-regenerate"
      @confirm="regenerate"
      @cancel="regenerateOpen = false"
      @backup="downloadIdentity"
    />

    <DestructiveConfirm
      :open="macOpen"
      title="Change the bridge's MAC address?"
      message="The address is the bridge id and the certificate's common name, so everything derived from it is rebuilt."
      confirm-label="Queue the MAC change"
      :require-text="confirmWord"
      :impacts="impactsMac"
      backup-label="Download identity bundle first"
      :backup-busy="backupBusy"
      backup-note="Offered, not taken automatically. It holds the private key."
      :busy="busy"
      testid="identity-mac"
      @confirm="changeMac"
      @cancel="macOpen = false"
      @backup="downloadIdentity"
    >
      <label
        class="fd-field"
        style="margin-top: var(--fd-3)"
      >
        <span class="fd-label">New MAC address</span>
        <input
          v-model="newMac"
          class="fd-input mono"
          type="text"
          placeholder="00:11:22:33:44:55"
          data-testid="identity-mac-input"
        >
      </label>
    </DestructiveConfirm>

    <DestructiveConfirm
      :open="resetOpen"
      title="Reinitialize the bridge state?"
      message="Everything the bridge built up from use is removed. The configuration, its servers and its tokens stay."
      confirm-label="Reinitialize state"
      :require-text="confirmWord"
      :impacts="impactsReset"
      backup-label="Download the state first"
      :backup-busy="backupBusy"
      backup-note="Offered, not taken automatically."
      :busy="busy"
      testid="identity-reset"
      @confirm="resetState"
      @cancel="resetOpen = false"
      @backup="downloadState"
    />

    <div
      class="fd-hr"
      style="margin: var(--fd-4) 0"
    />

    <div class="fd-between">
      <div>
        <div class="fd-panel__title">
          Restore an identity
        </div>
        <div class="fd-muted">
          Put back a certificate and pairing list from a bundle. The bundle never holds the z2m
          token or the admin password, so the restore asks for both.
        </div>
      </div>
      <button
        type="button"
        class="fd-btn fd-btn--sm"
        data-testid="identity-restore-open"
        @click="restoreOpen = !restoreOpen"
      >
        {{ restoreOpen ? 'Close' : 'Restore' }}
      </button>
    </div>

    <div
      v-if="restoreOpen"
      class="fd-stack"
      style="margin-top: var(--fd-3)"
      data-testid="identity-restore-form"
    >
      <label class="fd-field">
        <span class="fd-label">Identity bundle</span>
        <input
          type="file"
          accept="application/json,.json"
          class="fd-input"
          data-testid="identity-restore-file"
          @change="pickBundle"
        >
      </label>

      <label class="fd-field">
        <span class="fd-label">Admin password (authorises the restore)</span>
        <input
          v-model="restorePassword"
          type="password"
          autocomplete="current-password"
          class="fd-input"
          data-testid="identity-restore-password"
        >
      </label>

      <label class="fd-field">
        <span class="fd-label">z2m token (written into servers that have none)</span>
        <input
          v-model="restoreToken"
          type="password"
          autocomplete="off"
          class="fd-input"
          data-testid="identity-restore-token"
        >
      </label>

      <div class="fd-cluster">
        <button
          type="button"
          class="fd-btn fd-btn--danger"
          :disabled="busy || restoreBundle === null"
          data-testid="identity-restore-submit"
          @click="restore"
        >
          Restore identity
        </button>
        <span
          class="fd-muted"
          data-testid="identity-restore-state"
        >
          {{ restoreBundle === null ? 'Choose a bundle first.' : 'Bundle loaded.' }}
        </span>
      </div>
    </div>
  </section>
</template>
