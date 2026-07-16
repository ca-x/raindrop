import { expect, test } from "@playwright/test"

import { completeSetup, createCredentials, signIn } from "./support/app"
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
  await completeSetup(page, server, credentials)

  await page.getByRole("button", { name: "Sign out" }).click()
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible()

  await signIn(page, credentials)
  await page.getByRole("button", { name: "Sign out" }).click()
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible()

  await page.reload({ waitUntil: "domcontentloaded" })
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible()
  await expect(page.getByLabel(/Setup token/u)).toHaveCount(0)

  const bootstrap = await page.request.get(`${server.baseURL}/api/v1/bootstrap`)
  expect(bootstrap.status()).toBe(200)
  await expect(bootstrap.json()).resolves.toMatchObject({ status: "READY" })
})
