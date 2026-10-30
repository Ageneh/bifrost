<script setup lang="ts">
/**
 * Sign in, as a panel rather than a page.
 *
 * The shell is served unauthenticated, so this is what a visitor sees until
 * there is a session. A failure keeps the password field and says why.
 */
import { ref } from 'vue'
import { ApiError } from '~/lib/api'
import { useApi } from '~/composables/useApi'

const { login } = useApi()

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
    await login(password.value)
    password.value = ''
  }
  catch (error) {
    failure.value = error instanceof ApiError ? error.message : 'Sign-in failed.'
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
      data-testid="sign-in"
    >
      <div class="fd-panel__title">
        Sign in
      </div>
      <div class="fd-panel__sub">
        Bifrost Control
      </div>

      <form
        class="fd-stack"
        style="margin-top: var(--fd-4)"
        @submit.prevent="submit"
      >
        <label class="fd-field">
          <span class="fd-label">Password</span>
          <input
            v-model="password"
            class="fd-input"
            type="password"
            autocomplete="current-password"
            data-testid="sign-in-password"
          >
        </label>

        <div
          v-if="failure"
          class="fd-alert"
          role="alert"
          data-testid="sign-in-error"
        >
          {{ failure }}
        </div>

        <div class="fd-cluster">
          <button
            type="submit"
            class="fd-btn fd-btn--primary"
            :disabled="password === '' || busy"
            data-testid="sign-in-submit"
          >
            Sign in
          </button>
        </div>
      </form>

      <div
        class="fd-meta"
        style="margin-top: var(--fd-3)"
      >
        Opening Bifrost from the Home Assistant panel signs you in there.
      </div>
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
