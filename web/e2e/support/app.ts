import { randomBytes } from "node:crypto"

import { expect, type Locator, type Page } from "@playwright/test"

import type { ProductionServer } from "./productionServer"

export interface Credentials {
  username: string
  password: string
}

export function createCredentials(): Credentials {
  return {
    username: `Reader${randomBytes(4).toString("hex")}`,
    password: randomBytes(24).toString("base64url"),
  }
}

export async function completeSetup(
  page: Page,
  server: ProductionServer,
  credentials: Credentials,
): Promise<void> {
  await page.goto(server.baseURL, { waitUntil: "domcontentloaded" })
  await expect(page.getByRole("heading", { name: "Set up Raindrop" })).toBeVisible()
  await advanceToAdministrator(page, server)
  await completeAdministrator(page, credentials)
}

export async function advanceToAdministrator(
  page: Page,
  server: ProductionServer,
): Promise<void> {
  await fillSecret(page.getByLabel(/Setup token/u), server.setupToken)
  await page.getByRole("button", { name: "Check database and continue" }).click()
  await expect(
    page.getByRole("heading", { name: "Create the administrator" }),
  ).toBeVisible()
}

export async function completeAdministrator(
  page: Page,
  credentials: Credentials,
): Promise<void> {
  await page.getByLabel(/^Username/u).fill(credentials.username)
  await fillSecret(page.getByLabel(/^Password/u), credentials.password)
  await page.getByRole("button", { name: "Complete setup" }).click()
  await expectReaderReady(page)
}

export async function signIn(page: Page, credentials: Credentials): Promise<void> {
  await page.getByLabel("Username or email").fill(credentials.username)
  await fillSecret(page.getByLabel(/^Password/u), credentials.password)
  await page.getByRole("button", { name: "Sign in" }).click()
  await expectReaderReady(page)
}

export async function expectReaderReady(page: Page): Promise<void> {
  await expect(page).toHaveURL(/\/reader\/unread$/u)
  await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
  await expect(page.getByRole("button", { name: "Reload stored entries" })).toBeVisible()
}

async function fillSecret(locator: Locator, secret: string): Promise<void> {
  try {
    await locator.fill(secret)
  } catch {
    throw new Error("failed to populate a protected test field")
  }
}
