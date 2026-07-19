import { expect, test, type Page } from "@playwright/test"

import { completeSetup, createCredentials, signIn, signOut } from "./support/app"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer

test.beforeAll(async () => {
  server = await startProductionServer()
})

test.afterAll(async () => {
  await server?.stop()
})

test("production bundle completes setup, logs in, logs out, and keeps setup closed", async ({
  page,
}) => {
  const credentials = createCredentials()
  await page.goto(server.baseURL, { waitUntil: "domcontentloaded" })
  await expectSetupFitsTabletViewport(page)
  await page.setViewportSize({ width: 1280, height: 800 })
  await expectSetupContentInset(page)
  await completeSetup(page, server, credentials)

  await signOut(page)

  await signIn(page, credentials)
  await signOut(page)

  await page.reload({ waitUntil: "domcontentloaded" })
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible()
  await expect(page.getByLabel(/Setup token/u)).toHaveCount(0)

  const bootstrap = await page.request.get(`${server.baseURL}/api/v1/bootstrap`)
  expect(bootstrap.status()).toBe(200)
  await expect(bootstrap.json()).resolves.toMatchObject({ status: "READY" })
})

async function expectSetupFitsTabletViewport(page: Page) {
  await page.setViewportSize({ width: 768, height: 900 })
  const card = page.locator(".raindrop-auth-card")
  await expect(card).toBeVisible()
  const viewport = page.viewportSize()

  const [cardBox, viewportFits] = await Promise.all([
    card.boundingBox(),
    page.evaluate(() => {
      const frame = document.querySelector(".raindrop-auth-frame")!
      return (
        document.documentElement.scrollWidth <= window.innerWidth &&
        document.body.scrollWidth <= window.innerWidth &&
        frame.scrollWidth <= frame.clientWidth
      )
    }),
  ])
  expect(cardBox).not.toBeNull()
  expect(viewport).not.toBeNull()
  expect(viewportFits).toBe(true)
  expect(cardBox!.x).toBeGreaterThanOrEqual(16)
  expect(cardBox!.x + cardBox!.width).toBeLessThanOrEqual(viewport!.width - 16)
}

async function expectSetupContentInset(page: Page) {
  const card = page.locator(".raindrop-auth-card")
  const content = page.getByRole("heading", { name: "Connect a database" })
  await expect(card).toBeVisible()
  await expect(content).toBeVisible()

  const [cardBox, contentBox] = await Promise.all([card.boundingBox(), content.boundingBox()])
  expect(cardBox).not.toBeNull()
  expect(contentBox).not.toBeNull()
  expect(cardBox!.y).toBeGreaterThanOrEqual(23)
  expect(contentBox!.x - cardBox!.x).toBeGreaterThanOrEqual(24)
  expect(cardBox!.x + cardBox!.width - (contentBox!.x + contentBox!.width)).toBeGreaterThanOrEqual(
    24,
  )
}
