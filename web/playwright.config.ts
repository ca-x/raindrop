import { defineConfig } from "@playwright/test"

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ?? "/usr/bin/chromium"

export default defineConfig({
  testDir: "./e2e",
  outputDir: "./test-results",
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: [["line"]],
  use: {
    browserName: "chromium",
    launchOptions: { executablePath },
    locale: "en-US",
    screenshot: "off",
    trace: "off",
    video: "off",
  },
  projects: [
    {
      name: "desktop-production",
      testMatch: /setup-login\.spec\.ts/,
      use: { viewport: { width: 1280, height: 800 } },
    },
    {
      name: "mobile-390x844",
      testMatch: /mobile-foundation\.spec\.ts/,
      use: {
        viewport: { width: 390, height: 844 },
        deviceScaleFactor: 1,
        hasTouch: true,
        isMobile: true,
      },
    },
    {
      name: "mobile-360x800",
      testMatch: /mobile-foundation\.spec\.ts/,
      use: {
        viewport: { width: 360, height: 800 },
        deviceScaleFactor: 1,
        hasTouch: true,
        isMobile: true,
      },
    },
  ],
})
