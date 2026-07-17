import { defineConfig } from "@playwright/test"

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE

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
    ...(executablePath ? { launchOptions: { executablePath } } : {}),
    locale: "en-US",
    screenshot: "off",
    trace: "off",
    video: "off",
  },
  projects: [
    {
      name: "desktop-production",
      testMatch: /(setup-login|admin-only-setup)\.spec\.ts/,
      use: { viewport: { width: 1280, height: 800 } },
    },
    {
      name: "production-server-fixture",
      testMatch: /production-server\.spec\.ts/,
    },
    {
      name: "mobile-390x844",
      testMatch: /(mobile-foundation|admin-only-setup)\.spec\.ts/,
      use: {
        viewport: { width: 390, height: 844 },
        deviceScaleFactor: 1,
        hasTouch: true,
        isMobile: true,
      },
    },
    {
      name: "mobile-360x800",
      testMatch: /(mobile-foundation|admin-only-setup)\.spec\.ts/,
      use: {
        viewport: { width: 360, height: 800 },
        deviceScaleFactor: 1,
        hasTouch: true,
        isMobile: true,
      },
    },
    {
      name: "reader-1280x800",
      testMatch: /reader-workspace\.spec\.ts/,
      use: { viewport: { width: 1280, height: 800 } },
    },
    {
      name: "reader-900x800",
      testMatch: /reader-workspace\.spec\.ts/,
      use: { viewport: { width: 900, height: 800 } },
    },
    {
      name: "reader-390x844",
      testMatch: /reader-workspace\.spec\.ts/,
      use: {
        viewport: { width: 390, height: 844 },
        deviceScaleFactor: 1,
        hasTouch: true,
        isMobile: true,
      },
    },
    {
      name: "reader-360x800",
      testMatch: /reader-workspace\.spec\.ts/,
      use: {
        viewport: { width: 360, height: 800 },
        deviceScaleFactor: 1,
        hasTouch: true,
        isMobile: true,
      },
    },
  ],
})
