import { expect, test, type Page } from "@playwright/test"

import {
  advanceToAdministrator,
  completeAdministrator,
  createCredentials,
} from "./support/app"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer

test.beforeAll(async () => {
  server = await startProductionServer()
})

test.afterAll(async () => {
  await server?.stop()
})

test("mobile setup and authenticated shell preserve touch, keyboard, and history foundations", async ({
  page,
}) => {
  const credentials = createCredentials()
  await page.goto(server.baseURL, { waitUntil: "domcontentloaded" })
  await expect(page.getByRole("heading", { name: "Set up Raindrop" })).toBeVisible()
  await expectNoHorizontalScroll(page)
  await expectActionableTargets(page)
  await expectInViewport(page.getByLabel(/Setup token/u), page)
  await expectInViewport(page.getByLabel("Database URL"), page)
  await expectInViewport(
    page.getByRole("button", { name: "Check database and continue" }),
    page,
  )

  await advanceToAdministrator(page, server)
  await expectNoHorizontalScroll(page)
  await expectActionableTargets(page)
  await expectInViewport(page.getByLabel(/^Username/u), page)
  await expectInViewport(page.getByLabel(/^Email/u), page)
  await expectInViewport(page.getByLabel(/^Password/u), page)
  await expectInViewport(page.getByRole("button", { name: "Back to database" }), page)
  await expectInViewport(page.getByRole("button", { name: "Complete setup" }), page)

  await completeAdministrator(page, credentials)
  await expectNoHorizontalScroll(page)
  await expectActionableTargets(page)

  const menu = page.getByRole("button", { name: "Open sources" })
  await expect(menu).toBeVisible()
  await focusWithKeyboard(page, menu)
  await page.keyboard.press("Enter")
  const sources = page.getByRole("dialog", { name: "Sources" })
  await expect(sources).toBeVisible()
  await expect(
    sources.getByRole("heading", { name: `Raindrop · ${credentials.username}` }),
  ).toBeVisible()
  await expectActionableTargets(page)
  await page.keyboard.press("Escape")
  await expect(sources).not.toBeVisible()

  await menu.tap()
  await expect(sources).toBeVisible()
  await expectActionableTargets(page)
  await page.getByRole("button", { name: "Close navigation" }).tap()
  await expect(sources).not.toBeVisible()

  await menu.tap()
  await sources.getByText("All entries", { exact: true }).click()
  await expect(page).toHaveURL(`${server.baseURL}/reader/all`)
  await page.goBack()
  await expect(page).toHaveURL(`${server.baseURL}/reader/unread`)
  await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
})

async function expectNoHorizontalScroll(page: Page): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document.documentElement.scrollWidth <= window.innerWidth &&
          document.body.scrollWidth <= window.innerWidth,
      ),
    )
    .toBe(true)
}

async function expectActionableTargets(page: Page): Promise<void> {
  const failures = await page.locator(actionableSelector).evaluateAll((elements) => {
    const measured = new Set<HTMLElement>()
    return elements.flatMap((element) => {
      const target = effectiveTarget(element as HTMLElement)
      if (measured.has(target)) return []
      measured.add(target)
      const style = getComputedStyle(target)
      const box = target.getBoundingClientRect()
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        box.width === 0 ||
        box.height === 0
      ) {
        return []
      }
      if (box.width >= 44 && box.height >= 44) return []
      return [{
        name: target.getAttribute("aria-label") ?? target.textContent?.trim() ?? target.tagName,
        tag: target.tagName,
        type: target.getAttribute("type"),
        role: target.getAttribute("role"),
        width: Math.round(box.width),
        height: Math.round(box.height),
      }]
    })

    function effectiveTarget(target: HTMLElement): HTMLElement {
      if (target instanceof HTMLInputElement && target.type === "radio") {
        return target.closest<HTMLElement>(".astryx-radio-list-item") ?? target
      }
      if (target instanceof HTMLInputElement) {
        return target.closest<HTMLElement>(".astryx-text-input") ?? target
      }
      return target
    }
  })
  expect(failures).toEqual([])
}

async function expectInViewport(locator: ReturnType<Page["locator"]>, page: Page) {
  const box = await locator.boundingBox()
  const viewport = page.viewportSize()
  expect(box).not.toBeNull()
  expect(viewport).not.toBeNull()
  expect(box!.x).toBeGreaterThanOrEqual(0)
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport!.width)
  expect(box!.y).toBeGreaterThanOrEqual(0)
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport!.height)
}

async function focusWithKeyboard(page: Page, locator: ReturnType<Page["locator"]>) {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  for (let index = 0; index < 12; index += 1) {
    await page.keyboard.press("Tab")
    if (await locator.evaluate((element) => element === document.activeElement)) return
  }
  throw new Error("mobile navigation toggle is not keyboard reachable")
}

const actionableSelector = [
  "a[href]:not([data-testid='skip-to-content'])",
  "button",
  "input",
  "select",
  "textarea",
  "[role='button']",
  "[role='radio']",
  "[role='tab']",
  "[tabindex]:not([tabindex='-1'])",
].join(",")
