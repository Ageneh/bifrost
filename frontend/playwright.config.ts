import { defineConfig, devices } from '@playwright/test'

/**
 * The shell is checked at the width it has to survive: 390 px, the narrowest
 * phone the console is expected on, behind an ingress prefix like the one
 * Home Assistant serves it under.
 */
const PORT = 4319
const PREFIX = '/api/hassio_ingress/test'

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: `http://127.0.0.1:${PORT}${PREFIX}/`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'phone',
      use: {
        ...devices['iPhone 13'],
        // iPhone 13 is 390 x 844, the width the wave brief names
        viewport: { width: 390, height: 844 },
      },
    },
  ],
  webServer: {
    command: `node scripts/serve.mjs --port ${PORT} --prefix ${PREFIX}`,
    url: `http://127.0.0.1:${PORT}${PREFIX}/`,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
})
