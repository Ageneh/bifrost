// https://nuxt.com/docs/api/configuration/nuxt-config
//
// The build is a single-page application with hash routing: no server-side
// route fallback is needed, and the same output works at `/` on the LAN and
// under any Home Assistant ingress prefix. Asset references are relative to
// the document (`./_nuxt/...`) because the server injects a `<base href>`
// from the ingress path at request time.
export default defineNuxtConfig({
  modules: ['@nuxt/eslint'],
  ssr: false,
  devtools: { enabled: false },

  app: {
    baseURL: './',
    head: {
      title: 'Bifrost',
      meta: [
        { charset: 'utf-8' },
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        { name: 'color-scheme', content: 'dark' },
      ],
      link: [{ rel: 'icon', href: './favicon.svg', type: 'image/svg+xml' }],
    },
  },

  css: ['~/assets/css/tokens.css', '~/assets/css/components.css', '~/assets/css/base.css'],

  router: {
    options: {
      hashMode: true,
    },
  },
  compatibilityDate: '2025-07-15',

  typescript: {
    tsConfig: {
      // the Playwright config and the fixture server are tooling, not app code
      exclude: ['../e2e/**', '../playwright.config.ts', '../scripts/**'],
    },
  },

  eslint: {
    config: {
      stylistic: true,
    },
  },
})
