import { expect, test } from "@playwright/test"

import { createCredentials, expectReaderReady } from "./support/app"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer

test.beforeAll(async () => {
  server = await startProductionServer({ managedDatabase: true })
})

test.afterAll(async () => {
  await server?.stop()
})

test("managed empty database exposes only first-administrator setup", async ({ page }) => {
  const credentials = createCredentials()
  await page.goto(server.baseURL, { waitUntil: "domcontentloaded" })

  await expect(
    page.getByRole("heading", { name: "Create the administrator" }),
  ).toBeVisible()
  await expect(page.getByLabel(/Setup token/u)).toBeVisible()
  await expect(page.getByLabel(/Database URL/u)).toHaveCount(0)
  await expect(page.getByRole("radio", { name: "SQLite" })).toHaveCount(0)
  await expect(page.getByRole("button", { name: "Back to database" })).toHaveCount(0)
  await expect(page.getByText("1 / 1")).toBeVisible()

  await page.getByLabel(/Setup token/u).fill(server.setupToken)
  await page.getByLabel(/^Username/u).fill(credentials.username)
  await page.getByLabel(/^Password/u).fill(credentials.password)
  await page.getByRole("button", { name: "Complete setup" }).click()
  await expectReaderReady(page)

  const bootstrap = await page.request.get(`${server.baseURL}/api/v1/bootstrap`)
  await expect(bootstrap.json()).resolves.toEqual({
    status: "READY",
    version: "0.3.0",
  })
})
