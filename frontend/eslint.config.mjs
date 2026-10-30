// @ts-check
import withNuxt from './.nuxt/eslint.config.mjs'

export default withNuxt({
  ignores: ['lib/generated/**', '.output/**', '.nuxt/**', 'node_modules/**'],
})
