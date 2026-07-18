import { expect, test, type Page, type TestInfo } from "@playwright/test"

import { completeSetup, createCredentials } from "./support/app"
import {
  installReaderApiFixture,
  readerIds,
  type ReaderApiFixture,
} from "./support/readerApiFixture"
import {
  expectHostileContentContained,
  expectNoHorizontalOverflow,
  expectReducedMotion,
  expectScrollTop,
  readerRow,
  readerRowButton,
  setScrollTop,
} from "./support/readerAssertions"
import {
  verifyCompactCategoryRoute,
  verifyDirectCompactCategory,
  verifyMediumCategoryFocus,
  verifyWideCategoryWorkflow,
} from "./support/readerCategoryScenarios"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer

test.beforeAll(async () => {
  server = await startProductionServer()
})

test.afterAll(async () => {
  await server?.stop()
})

test("Reader workspace production contract", async ({ page }, testInfo) => {
  const fixture = await installReaderApiFixture(page)
  await completeSetup(page, server, createCredentials())
  await expectNoHorizontalOverflow(page)

  switch (testInfo.project.name) {
    case "reader-1280x800":
      await verifyWide(page, fixture)
      break
    case "reader-900x800":
      await verifyMedium(page, fixture)
      break
    case "reader-390x844":
      await verifyCompactHistory(page)
      break
    case "reader-360x800":
      await verifyDirectCompact(page)
      break
    default:
      throw new Error(`unexpected Reader project ${testInfo.project.name}`)
  }

  await verifyHostileDeepLink(page, testInfo)
})

async function verifyWide(page: Page, fixture: ReaderApiFixture): Promise<void> {
  await expect(page.getByRole("navigation", { name: "Sources" })).toBeVisible()
  await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
  await expect(page.getByRole("complementary", { name: "Article" })).toBeVisible()
  for (const key of ["J", "K", "N", "P"]) {
    await expect(page.getByRole("img", { name: key, exact: true })).toBeVisible()
  }

  await page.keyboard.press("n")
  await expect(readerRow(page, readerIds.firstEntry)).toHaveAttribute("aria-selected", "true")
  await expect(readerRowButton(page, readerIds.firstEntry)).toBeFocused()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread`)
  expect(fixture.patches).toHaveLength(0)

  await page.keyboard.press("j")
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread/entry/${readerIds.secondEntry}`)
  await expect(page.getByRole("heading", { name: "Second quiet article" })).toBeVisible()
  for (const key of ["M", "S"]) {
    await expect(page.getByRole("img", { name: key, exact: true })).toBeVisible()
  }
  await expect.poll(() => fixture.entryState(readerIds.secondEntry).isRead).toBe(true)
  await page.keyboard.press("k")
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread/entry/${readerIds.firstEntry}`)
  await page.goBack()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread`)

  await page.keyboard.press("m")
  await page.keyboard.press("s")
  await expect.poll(() => fixture.entryState(readerIds.firstEntry)).toEqual({
    entryId: readerIds.firstEntry,
    isRead: false,
    isStarred: true,
  })

  await page.getByRole("button", { name: "Add subscription" }).click()
  const addDialog = page.getByRole("dialog").filter({
    has: page.getByRole("heading", { name: "Add subscription" }),
  })
  await expect(addDialog).toBeVisible()
  const beforeDialog = { url: page.url(), patches: fixture.patches.length }
  await addDialog.getByRole("button", { name: "Cancel" }).focus()
  await page.keyboard.press("j")
  await page.keyboard.press("m")
  expect({ url: page.url(), patches: fixture.patches.length }).toEqual(beforeDialog)
  await addDialog.getByRole("button", { name: "Cancel" }).click()

  await readerRowButton(page, readerIds.firstEntry).click()
  const articleUrl = page.url()
  const queue = page.getByTestId("entry-queue-scroll")
  await setScrollTop(queue, 260)
  fixture.discoverPending()
  await page.getByRole("button", { name: "Reload stored entries" }).click()
  const pending = page.getByRole("status").filter({ hasText: "1 new entries available" })
  await expect(pending).toBeVisible()
  await pending.getByRole("button", { name: "Show 1 new entries" }).click()
  await expect(page).toHaveURL(articleUrl)
  await expectScrollTop(queue, 0)
  await expect(readerRow(page, readerIds.pendingEntry)).toHaveAttribute("aria-selected", "true")
  await expect(readerRowButton(page, readerIds.pendingEntry)).toBeFocused()

  const article = page.getByRole("article")
  await readerRowButton(page, readerIds.firstEntry).click()
  await setScrollTop(article, 320)
  await readerRowButton(page, readerIds.secondEntry).click()
  await expectScrollTop(article, 0)
  await setScrollTop(article, 180)
  await readerRowButton(page, readerIds.firstEntry).click()
  await expectScrollTop(article, 320)
  await verifyWideCategoryWorkflow(page, fixture, server.baseURL)
  await expectNoHorizontalOverflow(page)
}

async function verifyMedium(page: Page, fixture: ReaderApiFixture): Promise<void> {
  await expect(page.getByRole("navigation", { name: "Sources" })).toHaveCount(0)
  await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
  await expect(page.getByRole("complementary", { name: "Article" })).toBeVisible()
  const menu = page.getByRole("button", { name: "Open sources" })
  await menu.click()
  const sources = page.getByRole("dialog", { name: "Sources" })
  await expect(sources).toBeVisible()
  const before = { url: page.url(), patches: fixture.patches.length }
  await page.keyboard.press("j")
  await page.keyboard.press("m")
  expect({ url: page.url(), patches: fixture.patches.length }).toEqual(before)
  await page.keyboard.press("Escape")
  await expect(sources).not.toBeVisible()
  await expect(menu).toBeFocused()

  await menu.click()
  await sources.getByText("Quiet Web", { exact: true }).click()
  await expect(page).toHaveURL(`${server.baseURL}/reader/feed/${readerIds.feedA}`)
  await menu.click()
  await sources.getByText("Rust Dispatch", { exact: true }).click()
  await expect(page).toHaveURL(`${server.baseURL}/reader/feed/${readerIds.feedB}`)
  await verifyMediumCategoryFocus(page)
  await expectNoHorizontalOverflow(page)
}

async function verifyCompactHistory(page: Page): Promise<void> {
  const queue = page.getByTestId("entry-queue-scroll")
  const restoredOffset = await setScrollTop(queue, 260)
  await readerRowButton(page, readerIds.fourthEntry).click()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread/entry/${readerIds.fourthEntry}`)
  await expect(page.getByRole("heading", { name: "Fixture entry 04" })).toBeFocused()
  await page.getByRole("button", { name: "Back to entry queue" }).click()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread`)
  await expectScrollTop(page.getByTestId("entry-queue-scroll"), restoredOffset)
  await expect(readerRowButton(page, readerIds.fourthEntry)).toBeFocused()
  await page.goForward()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread/entry/${readerIds.fourthEntry}`)
  await page.goBack()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread`)
  await verifyCompactCategoryRoute(page, server.baseURL)
}

async function verifyDirectCompact(page: Page): Promise<void> {
  await verifyDirectCompactCategory(page, server.baseURL)
}

async function verifyHostileDeepLink(page: Page, testInfo: TestInfo): Promise<void> {
  await page.goto(`${server.baseURL}/reader/unread/entry/${readerIds.deepOnlyEntry}`)
  await expect(page.getByRole("heading", { name: "Hostile deep-link article" })).toBeVisible()
  await expect(page.getByRole("article")).toBeVisible()
  await expectHostileContentContained(page)
  if (testInfo.project.name === "reader-360x800") await expectReducedMotion(page)
}
