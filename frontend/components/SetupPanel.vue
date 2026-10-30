<script setup lang="ts">
/**
 * The first password.
 *
 * Direct on the LAN this needs the one-time setup code the bridge printed to
 * its console; through the Home Assistant panel there is no code, because
 * Home Assistant already authenticated the user.
 */
import { ref } from 'vue'
import { ApiError } from '~/lib/api'
import { useApi } from '~/composables/useApi'

const { setup, ingress } = useApi()

const setupCode = ref('')
const password = ref('')
const busy = ref(false)
const failure = ref<string | null>(null)

async function submit(): Promise<void> {
  if (password.value === '' || busy.value) {
    return
  }
  busy.value = true
  failure.value = null
  try {
    await setup(password.value, ingress.value ? undefined : setupCode.value)
    password.value = ''
    setupCode.value = ''
  }
  catch (error) {
    failure.value = error instanceof ApiError ? error.message : 'Setting the password failed.'
  }
  finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="gate">
    <div
      class="fd-panel gate__panel"
      data-testid="setup-panel"
    >
      <div class="fd-panel__title">
        First run
      </div>
      <div class="fd-panel__sub">
        Set the password for Bifrost Control.
        <template v-if="!ingress">
          The setup code is in the bridge's console output.
        </template>
      </div>

      <form
        class="fd-stack"
        style="margin-top: var(--fd-4)"
        @submit.prevent="submit"
      >
        <label
          v-if="!ingress"
          class="fd-field"
        >
          <span class="fd-label">Setup code</span>
          <input
            v-model="setupCode"
            class="fd-input"
            type="text"
            autocomplete="one-time-code"
            data-testid="setup-code"
          >
        </label>

        <label class="fd-field">
          <span class="fd-label">New password</span>
          <input
            v-model="password"
            class="fd-input"
            type="password"
            autocomplete="new-password"
            data-testid="setup-password"
          >
        </label>

        <div
          v-if="failure"
          class="fd-alert"
          role="alert"
          data-testid="setup-error"
        >
          {{ failure }}
        </div>

        <div class="fd-cluster">
          <button
            type="submit"
            class="fd-btn fd-btn--primary"
            :disabled="password === '' || busy"
            data-testid="setup-submit"
          >
            Set password
          </button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.gate {
  display: grid;
  place-items: center;
  min-height: 100%;
  padding: var(--fd-5);
}
.gate__panel {
  width: min(420px, 100%);
}
</style>
