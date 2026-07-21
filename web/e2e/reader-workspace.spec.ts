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
import {
  verifyCompactPreferences,
  verifyMediumPreferencesFocus,
  verifyWidePreferences,
} from "./support/readerPreferenceScenarios"
import {
  verifyCompactQueueMenu,
  verifyMediumFeedSearchAndMenu,
  verifyStableSnapshotBulkRead,
  verifyWideUnreadSourceHotkeys,
} from "./support/readerEfficiencyScenarios"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer

test.beforeAll(async () => {
  server = await startProductionServer()
})

test.afterAll(async () => {
  await server?.stop()
})

test("Reader workspace production contract", async ({ page }, testInfo) => {
  testInfo.setTimeout(60_000)
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
      await verifyCompactHistory(page, fixture)
      break
    case "reader-360x800":
      await verifyDirectCompact(page, fixture)
      break
    default:
      throw new Error(`unexpected Reader project ${testInfo.project.name}`)
  }

  await verifyHostileDeepLink(page, testInfo)
})

test("Reader stable snapshot bulk read", async ({ page }, testInfo) => {
  test.skip(
    testInfo.project.name !== "reader-1280x800",
    "Stable snapshot transaction is exercised once in the wide Reader project.",
  )
  const transactionServer = await startProductionServer()
  try {
    const fixture = await installReaderApiFixture(page)
    fixture.discoverPending()
    await completeSetup(page, transactionServer, createCredentials())
    await verifyStableSnapshotBulkRead(page, fixture, transactionServer.baseURL)
  } finally {
    await transactionServer.stop()
  }
})

test("Reader refresh observability", async ({ page }, testInfo) => {
  testInfo.setTimeout(60_000)
  const refreshServer = await startProductionServer()
  try {
    const fixture = await installReaderApiFixture(page)
    switch (testInfo.project.name) {
      case "reader-900x800":
        fixture.organization.setRefreshState(readerIds.subscriptionA, "DEGRADED")
        break
      case "reader-390x844":
        fixture.organization.setRefreshState(readerIds.subscriptionA, "BACKING_OFF")
        break
      case "reader-360x800":
        fixture.organization.setRefreshState(readerIds.subscriptionA, "ERROR")
        break
    }
    await completeSetup(page, refreshServer, createCredentials())
    const sources = await selectQuietWebAndShowSources(page, refreshServer.baseURL)
    const refreshButton = sources.getByRole("button", { name: "Refresh Quiet Web" })
    await expectTouchTarget(refreshButton)

    switch (testInfo.project.name) {
      case "reader-1280x800":
        await refreshButton.click()
        await expect(sources.getByText("Waiting for a refresh worker.")).toBeVisible()
        await expect(refreshButton).toHaveAttribute("aria-disabled", "true")
        await expect(sources.getByText("Fetching and processing feed updates.")).toBeVisible({
          timeout: 3_000,
        })
        await expect(sources.getByText(/Last successful refresh:/)).toBeVisible({
          timeout: 3_000,
        })
        break
      case "reader-900x800":
        await expect(sources.getByRole("alert")).toContainText(
          "2 duplicate entries were ignored.",
        )
        break
      case "reader-390x844":
        await expect(sources.getByRole("alert")).toContainText("Refresh cooling down")
        await expect(sources.getByRole("alert")).toContainText("Next attempt after")
        break
      case "reader-360x800":
        await expect(sources.getByRole("alert")).toContainText("Refresh failed")
        await expect(sources.getByRole("alert")).toContainText("Last successful refresh:")
        break
      default:
        throw new Error(`unexpected Reader project ${testInfo.project.name}`)
    }
    await expectNoHorizontalOverflow(page)
  } finally {
    await refreshServer.stop()
  }
})

async function verifyWide(page: Page, fixture: ReaderApiFixture): Promise<void> {
  await expect(page.getByRole("navigation", { name: "Sources" })).toBeVisible()
  await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
  await expect(page.getByRole("complementary", { name: "Article" })).toBeVisible()
  for (const key of ["J", "K", "N", "P"]) {
    await expect(page.getByRole("img", { name: key, exact: true })).toBeVisible()
  }
  await verifyWideUnreadSourceHotkeys(page, server.baseURL)

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

  await page.getByRole("button", { name: "Manage subscriptions" }).click()
  const addDialog = page.getByRole("dialog", { name: "Manage subscriptions" })
  await expect(addDialog).toBeVisible()
  const beforeDialog = { url: page.url(), patches: fixture.patches.length }
  await addDialog.getByRole("button", { name: "Close" }).focus()
  await page.keyboard.press("j")
  await page.keyboard.press("m")
  expect({ url: page.url(), patches: fixture.patches.length }).toEqual(beforeDialog)
  await addDialog.getByRole("button", { name: "Close" }).click()

  await readerRowButton(page, readerIds.firstEntry).click()
  await expect.poll(() => fixture.entryState(readerIds.firstEntry).isRead).toBe(true)
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
  const managementDialog = page.getByRole("dialog", { name: "Manage subscriptions" })
  await managementDialog.getByRole("button", { name: "Close" }).click()
  await expect(managementDialog).not.toBeVisible()
  await readerRowButton(page, readerIds.firstEntry).click()
  await expect(page.getByRole("heading", { name: "First quiet article" })).toBeVisible()
  await verifyWidePreferences(page, fixture)
  await expectNoHorizontalOverflow(page)
}

async function selectQuietWebAndShowSources(page: Page, baseURL: string) {
  const navigation = page.getByRole("navigation", { name: "Sources" })
  if (await navigation.count()) {
    await navigation.getByRole("button", { name: "Quiet Web" }).click()
    await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedA}`)
    return navigation
  }

  const openSources = page.getByRole("button", { name: "Open sources" })
  await openSources.click()
  let sources = page.getByRole("dialog", { name: "Sources" })
  await sources.getByRole("button", { name: "Quiet Web" }).click()
  await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedA}`)
  await openSources.click()
  sources = page.getByRole("dialog", { name: "Sources" })
  await expect(sources).toBeVisible()
  return sources
}

async function expectTouchTarget(locator: ReturnType<Page["getByRole"]>): Promise<void> {
  const box = await locator.boundingBox()
  expect(box?.width ?? 0).toBeGreaterThanOrEqual(44)
  expect(box?.height ?? 0).toBeGreaterThanOrEqual(44)
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

  await verifyMediumFeedSearchAndMenu(page, fixture, server.baseURL)
  await verifyMediumCategoryFocus(page)
  await verifyMediumPreferencesFocus(page)
  await expectNoHorizontalOverflow(page)
}

async function verifyCompactHistory(
  page: Page,
  fixture: ReaderApiFixture,
): Promise<void> {
  await verifyCompactQueueMenu(page)
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
  await verifyCompactPreferences(page, fixture, false)
}

async function verifyDirectCompact(
  page: Page,
  fixture: ReaderApiFixture,
): Promise<void> {
  await verifyCompactQueueMenu(page)
  await verifyDirectCompactCategory(page, server.baseURL)
  await verifyCompactPreferences(page, fixture, true)
}

async function verifyHostileDeepLink(page: Page, testInfo: TestInfo): Promise<void> {
  await page.goto(`${server.baseURL}/reader/unread/entry/${readerIds.deepOnlyEntry}`)
  await expect(page.getByRole("heading", { name: "Hostile deep-link article" })).toBeVisible()
  await expect(page.getByRole("article")).toBeVisible()
  await expectHostileContentContained(page)
  if (testInfo.project.name === "reader-360x800") await expectReducedMotion(page)
}
