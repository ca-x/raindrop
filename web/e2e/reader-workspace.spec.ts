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

test("Expanded source tree keeps hover distinct and reaches the final feed", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name !== "reader-1280x800",
    "The constrained desktop source pane is exercised once in the wide project.",
  )
  const overflowServer = await startProductionServer()
  try {
    const fixture = await installReaderApiFixture(page)
    const overflowCount = 18
    for (let index = 0; index < overflowCount; index += 1) {
      const suffix = (600 + index).toString(16).padStart(12, "0")
      const categoryId = `00000000-0000-4000-8000-${suffix}`
      const feedId = `00000000-0000-4000-9000-${suffix}`
      const subscriptionId = `00000000-0000-4000-a000-${suffix}`
      fixture.organization.categories.push({
        categoryId,
        title: `Overflow category ${String(index + 1).padStart(2, "0")}`,
        position: (index + 2) * 1024,
      })
      fixture.organization.subscriptions.push({
        subscriptionId,
        feedId,
        categoryId,
        titleOverride: null,
        position: 0,
        title: `Overflow feed ${String(index + 1).padStart(2, "0")}`,
        feedUrl: `https://overflow-${index + 1}.example/feed.xml`,
        siteUrl: `https://overflow-${index + 1}.example/`,
        unreadCount: index + 1,
        refresh: null,
      })
    }

    await completeSetup(page, overflowServer, createCredentials())
    const sources = page.getByRole("navigation", { name: "Sources" })
    const tree = sources.locator(".reader-source-list > [role='tree']")
    await expect.poll(() => tree.evaluate((element) => element.scrollHeight > element.clientHeight))
      .toBe(true)

    await sources.getByRole("button", { name: "Rust Dispatch" }).click()
    const selectedRow = sources
      .getByRole("treeitem", { name: /Rust Dispatch/u })
      .locator(":scope > div:last-of-type > div")
    const hoveredRow = sources
      .getByRole("treeitem", { name: /Engineering/u })
      .locator(":scope > div:last-of-type > div")
    await hoveredRow.hover()
    const [selectedStyle, hoverStyle] = await Promise.all([
      selectedRow.evaluate((element) => {
        const style = getComputedStyle(element)
        return { background: style.backgroundColor, shadow: style.boxShadow }
      }),
      hoveredRow.evaluate((element) => {
        const style = getComputedStyle(element)
        return { background: style.backgroundColor, shadow: style.boxShadow }
      }),
    ])
    expect(hoverStyle.background).not.toBe(selectedStyle.background)
    expect(selectedStyle.shadow).not.toBe("none")
    expect(hoverStyle.shadow).not.toBe("none")

    const queueRows = page.locator(".reader-entry-item")
    await queueRows.nth(0).getByRole("button").click()
    await queueRows.nth(1).hover()
    const [selectedEntryStyle, hoverEntryStyle] = await Promise.all([
      queueRows.nth(0).evaluate((element) => {
        const style = getComputedStyle(element)
        return { background: style.backgroundColor, shadow: style.boxShadow }
      }),
      queueRows.nth(1).evaluate((element) => {
        const style = getComputedStyle(element)
        return { background: style.backgroundColor, shadow: style.boxShadow }
      }),
    ])
    expect(hoverEntryStyle.background).not.toBe(selectedEntryStyle.background)
    expect(selectedEntryStyle.shadow).not.toBe("none")
    expect(hoverEntryStyle.shadow).not.toBe("none")

    const finalFeed = sources.getByRole("button", { name: "Overflow feed 18" })
    await tree.evaluate((element) => {
      element.scrollTop = element.scrollHeight
    })
    await expect(finalFeed).toBeInViewport()
    await finalFeed.click()
    await expect(page).toHaveURL(/\/reader\/feed\/00000000-0000-4000-9000-/u)
  } finally {
    await overflowServer.stop()
  }
})

test("Reader lists respect every layout density", async ({ page }, testInfo) => {
  test.skip(
    testInfo.project.name !== "reader-1280x800",
    "Reader density is measured once with a fine pointer in the wide project.",
  )
  const densityServer = await startProductionServer()
  try {
    const fixture = await installReaderApiFixture(page)
    await completeSetup(page, densityServer, createCredentials())
    const sources = page.getByRole("navigation", { name: "Sources" })
    const tree = sources.locator(".reader-source-list")
    const feedRow = sources.locator(
      `[data-tree-id="feed:${readerIds.feedB}"] > div:last-of-type > div`,
    )
    const icon = feedRow.locator(".reader-source-icon")
    const entryRow = page.locator(".reader-entry-item").first()

    for (const [layoutDensity, density, rowBlockSize, iconSize, entryBlockSize] of [
      ["COMPACT", "compact", 28, 14, 64],
      ["BALANCED", "balanced", 36, 16, 76],
      ["SPACIOUS", "spacious", 44, 18, 88],
    ] as const) {
      fixture.preferences.setCurrent({
        ...fixture.preferences.current(),
        layoutDensity,
      })
      await page.evaluate(() => localStorage.clear())
      await page.reload({ waitUntil: "domcontentloaded" })
      await expect(tree).toHaveAttribute("data-density", density)
      await expect.poll(async () => ({
        rowBlockSize: Math.round(
          await feedRow.evaluate((element) => element.getBoundingClientRect().height),
        ),
        iconSize: Math.round(
          await icon.evaluate((element) => element.getBoundingClientRect().width),
        ),
        entryBlockSize: Math.round(
          await entryRow.evaluate((element) => element.getBoundingClientRect().height),
        ),
      })).toEqual({ rowBlockSize, iconSize, entryBlockSize })
    }
  } finally {
    await densityServer.stop()
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

test("Selected text translation waits for an explicit floating action", async ({
  page,
}, testInfo) => {
  test.skip(
    !["reader-1280x800", "reader-390x844"].includes(testInfo.project.name),
    "Selection translation is exercised once per pointer family.",
  )
  const selectionServer = await startProductionServer()
  try {
    const translation = await installSelectionTranslationFixture(page)
    await installReaderApiFixture(page)
    await completeSetup(page, selectionServer, createCredentials())
    await readerRowButton(page, readerIds.firstEntry).click()
    await expect(page.getByRole("heading", { name: "First quiet article" })).toBeVisible()

    const paragraph = page.locator(".reader-article-body p").first()
    const selectionPoint = await paragraph.evaluate((element) => {
      const range = document.createRange()
      range.selectNodeContents(element)
      const selection = document.getSelection()
      selection?.removeAllRanges()
      selection?.addRange(range)
      const rectangle = range.getBoundingClientRect()
      return { clientX: rectangle.right, clientY: rectangle.bottom }
    })
    await paragraph.dispatchEvent("pointerup", {
      bubbles: true,
      ...selectionPoint,
      pointerType: testInfo.project.name.includes("390") ? "touch" : "mouse",
    })

    const action = page.getByRole("button", {
      name: "Look up or translate selected text",
    })
    await expect(action).toBeVisible()
    expect(translation.requests).toEqual([])
    const contextMenuWasPrevented = await paragraph.evaluate((element) => {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true })
      element.dispatchEvent(event)
      return event.defaultPrevented
    })
    expect(contextMenuWasPrevented).toBe(false)
    if (testInfo.project.name === "reader-390x844") {
      const box = await action.boundingBox()
      expect(box?.width ?? 0).toBeGreaterThanOrEqual(44)
      expect(box?.height ?? 0).toBeGreaterThanOrEqual(44)
    }

    await action.click()
    const dialog = page.getByRole("dialog", { name: "Selected text translation" })
    const popover = dialog.locator("xpath=ancestor::*[@popover][1]")
    await expect
      .poll(() => popover.evaluate((element) => element.matches(":popover-open")))
      .toBe(true)
    await expect(dialog).toBeVisible()
    if (testInfo.project.name === "reader-390x844") {
      const viewport = page.viewportSize()
      const box = await popover.boundingBox()
      expect(box?.x ?? 0).toBeGreaterThanOrEqual(16)
      expect(
        (viewport?.width ?? 0) - ((box?.x ?? 0) + (box?.width ?? 0)),
      ).toBeGreaterThanOrEqual(16)
    }
    await expect(dialog.getByText("确定性阅读条目 1", { exact: true })).toBeVisible()
    expect(translation.requests).toEqual(["Deterministic Reader entry 1"])
    expect(translation.hasCsrf).toBe(true)
  } finally {
    await selectionServer.stop()
  }
})

test("DeepLX article translation renders progressive segments", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name !== "reader-1280x800",
    "Progressive article rendering is exercised once in the wide Reader project.",
  )
  const progressiveServer = await startProductionServer()
  try {
    await installSelectionTranslationFixture(page)
    await installReaderApiFixture(page)
    await installProgressiveArticleTranslationStream(page)
    await completeSetup(page, progressiveServer, createCredentials())
    await readerRowButton(page, readerIds.firstEntry).click()
    await expect(page.getByRole("heading", { name: "First quiet article" })).toBeVisible()

    await page.getByRole("button", { name: "Translate article" }).click()
    await expect(page.getByText("确定性阅读条目 1", { exact: true })).toBeVisible()
    await expect(page.getByText("Translating 1/2", { exact: true })).toBeVisible()
    await expect(page.getByText("可滚动段落 1", { exact: true })).not.toBeVisible()
    await expect(page.getByText("可滚动段落 1", { exact: true })).toBeVisible()
    await expect(page.getByText("Translated by DeepLX", { exact: true })).toBeVisible()
  } finally {
    await progressiveServer.stop()
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
  await expect.poll(() => readerRow(page, readerIds.firstEntry).evaluate((element) => {
    const style = getComputedStyle(element)
    return { outlineStyle: style.outlineStyle, outlineWidth: style.outlineWidth }
  })).toEqual({ outlineStyle: "solid", outlineWidth: "2px" })
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
  await readerRowButton(page, readerIds.fourthEntry).click()
  await expect(page.getByRole("heading", { name: "Fixture entry 04" })).toBeVisible()
  await verifyWidePreferences(page, fixture)
  await expectNoHorizontalOverflow(page)
}

async function selectQuietWebAndShowSources(page: Page, baseURL: string) {
  const navigation = page.getByRole("navigation", { name: "Sources" })
  if (await navigation.count()) {
    await navigation.getByRole("button", { name: "Quiet Web", exact: true }).click()
    await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedA}`)
    return navigation
  }

  const openSources = page.getByRole("button", { name: "Open sources" })
  await openSources.click()
  let sources = page.getByRole("dialog", { name: "Sources" })
  await sources.getByRole("button", { name: "Quiet Web", exact: true }).click()
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

async function installSelectionTranslationFixture(page: Page): Promise<{
  requests: string[]
  hasCsrf: boolean
}> {
  const state = { requests: [] as string[], hasCsrf: false }
  await page.route("**/api/v3/plugins/translation**", async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.pathname === "/api/v3/plugins/translation" && request.method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          engine: "DEEPLX",
          displayMode: "BILINGUAL",
          isEnabled: true,
          defaultTargetLocale: "zh-CN",
          openAi: {
            providerId: null,
            maxOutputTokens: 2048,
            profile: "GENERAL",
            customSystemPrompt: null,
            customPrompt: null,
          },
          deepLx: {
            displayName: "DeepLX",
            description: null,
            baseUrl: null,
            isProgressive: true,
            hasApiKey: false,
          },
          revision: 1,
        }),
      })
      return
    }
    if (url.pathname === "/api/v3/plugins/translation/lookup" && request.method() === "POST") {
      const body = request.postDataJSON() as { text: string }
      state.requests.push(body.text)
      state.hasCsrf = Boolean(request.headers()["x-csrf-token"])
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          query: body.text,
          translation: "确定性阅读条目 1",
          definition: "用于验证选区查词的确定性结果。",
          examples: [],
          providerLabel: "DeepLX",
          detectedSourceLocale: "en",
          targetLocale: "zh-CN",
        }),
      })
      return
    }
    throw new Error(`unexpected Translation request: ${request.method()} ${url.pathname}`)
  })
  return state
}

async function installProgressiveArticleTranslationStream(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const originalFetch = window.fetch.bind(window)
    window.fetch = async (input, init) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url
      if (!url.endsWith("/translate/progressive")) return originalFetch(input, init)
      const encoder = new TextEncoder()
      const event = (kind: string, patch: Record<string, unknown>) =>
        `${JSON.stringify({
          kind,
          title: null,
          segment: null,
          providerLabel: null,
          detectedSourceLocale: null,
          targetLocale: null,
          completedSegments: 0,
          totalSegments: 0,
          error: null,
          ...patch,
        })}\n`
      return new Response(
        new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(encoder.encode([
              event("STARTED", { targetLocale: "zh-CN", totalSegments: 2 }),
              event("TITLE", {
                title: "安静的第一篇文章",
                providerLabel: "DeepLX",
                detectedSourceLocale: "en",
                targetLocale: "zh-CN",
                totalSegments: 2,
              }),
              event("SEGMENT", {
                segment: {
                  index: 0,
                  originalText: "Deterministic Reader entry 1",
                  translatedText: "确定性阅读条目 1",
                },
                completedSegments: 1,
                totalSegments: 2,
              }),
            ].join("")))
            window.setTimeout(() => {
              controller.enqueue(encoder.encode([
                event("SEGMENT", {
                  segment: {
                    index: 1,
                    originalText: "Scrollable paragraph 1",
                    translatedText: "可滚动段落 1",
                  },
                  completedSegments: 2,
                  totalSegments: 2,
                }),
                event("COMPLETED", {
                  completedSegments: 2,
                  totalSegments: 2,
                }),
              ].join("")))
              controller.close()
            }, 1_500)
          },
        }),
        {
          status: 200,
          headers: { "content-type": "application/x-ndjson; charset=utf-8" },
        },
      )
    }
  })
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
