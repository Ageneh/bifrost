<script setup lang="ts">
/**
 * System · Bridge and network.
 *
 * Reads the running configuration and the draft, edits the bridge block into
 * the draft, and says what each field costs before it is saved. Nothing here
 * writes the active configuration: Apply, in the draft bar, does that, and
 * the blast radius on every field is the answer to "what will that do".
 */
import { computed, onMounted, ref } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import BridgeForm from '~/components/BridgeForm.vue'
import StatusPill from '~/components/StatusPill.vue'
import SystemTabs from '~/components/SystemTabs.vue'
import { useApi } from '~/composables/useApi'
import { useDraft } from '~/composables/useDraft'
import { useToast } from '~/composables/useToast'
import type { AppConfig } from '~/lib/generated/AppConfig'
import type { BridgeConfig } from '~/lib/generated/BridgeConfig'
import { isObject } from '~/lib/json'

const { client } = useApi()
const draft = useDraft()
const toast = useToast()

const active = ref<AppConfig | null>(null)
const loading = ref(true)
const failure = ref<string | null>(null)
const saving = ref(false)

const activeBridge = computed<BridgeConfig | null>(() => active.value?.bridge ?? null)

const draftBridge = computed<BridgeConfig | null>(() => {
  const bridge: unknown = draft.document.value?.bridge
  return isObject(bridge) ? (bridge as unknown as BridgeConfig) : null
})

/** The draft wins when there is one: that is what Apply will write. */
const model = computed<BridgeConfig | null>(() => draftBridge.value ?? activeBridge.value)

const bridgeFindings = computed(() =>
  draft.validation.value.filter(finding => finding.path.startsWith('bridge.')),
)

const sourceLabel = computed(() => (draftBridge.value === null ? 'running' : 'draft'))

async function load(): Promise<void> {
  loading.value = true
  try {
    const config = await client.activeConfig()
    active.value = config as unknown as AppConfig
    failure.value = null
  }
  catch (cause) {
    failure.value = cause instanceof Error ? cause.message : 'Could not read the bridge state.'
  }
  finally {
    loading.value = false
  }
  await draft.load()
}

onMounted(load)

async function save(patch: Record<string, string | number>): Promise<void> {
  saving.value = true
  try {
    const written = await draft.patch({ bridge: patch })
    if (written) {
      toast.ok('Saved to the draft. Apply the draft to put it into effect.')
    }
    else if (draft.error.value) {
      toast.fail(draft.error.value)
    }
  }
  finally {
    saving.value = false
  }
}

/** Re-read the draft after another browser moved it. */
async function reloadDraft(): Promise<void> {
  await draft.load()
  toast.ok('Draft reloaded.')
}
</script>

<template>
  <div
    class="fd-stack"
    data-testid="bridge-screen"
  >
    <SystemTabs current="bridge" />

    <AsyncState
      :loading="loading && activeBridge === null"
      :error="activeBridge === null ? failure : null"
      :skeleton-rows="5"
      testid="bridge-state"
      @retry="load"
    >
      <template v-if="activeBridge !== null && model !== null">
        <div
          class="fd-panel"
          data-testid="bridge-panel"
        >
          <div class="fd-panel__head">
            <div>
              <div class="fd-panel__title">
                Bridge &amp; network
              </div>
              <div class="fd-panel__sub">
                Editing the {{ sourceLabel }} configuration. Every field says what saving it costs;
                nothing takes effect until the draft is applied.
              </div>
            </div>
            <StatusPill
              :label="draftBridge === null ? 'running config' : 'draft'"
              :state="draftBridge === null ? 'ok' : 'warn'"
              testid="bridge-source"
            />
          </div>

          <div
            v-if="draft.status.value === 'conflict'"
            class="fd-notice"
            data-testid="bridge-conflict"
            role="status"
          >
            Another browser changed the draft, so this save did not land. Reload the draft to see
            its revision, then edit again — the values in this form are not on the bridge yet.
            <button
              type="button"
              class="fd-btn fd-btn--sm"
              style="margin-left: var(--fd-2)"
              data-testid="bridge-reload"
              @click="reloadDraft"
            >
              Reload draft
            </button>
          </div>

          <div
            v-if="bridgeFindings.length > 0"
            class="fd-alert"
            data-testid="bridge-findings"
            role="alert"
          >
            {{ bridgeFindings.length }} finding{{ bridgeFindings.length === 1 ? '' : 's' }} on these
            settings. The draft cannot be applied until the errors are fixed.
          </div>

          <BridgeForm
            :active="activeBridge"
            :model="model"
            :validation="bridgeFindings"
            :busy="saving"
            testid="bridge-form"
            @save="save"
          />
        </div>
      </template>
    </AsyncState>
  </div>
</template>
