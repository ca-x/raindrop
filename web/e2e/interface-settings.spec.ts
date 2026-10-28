import { expect, test, type Page } from "@playwright/test"

import { completeSetup, createCredentials } from "./support/app"
import { installReaderApiFixture, readerIds } from "./support/readerApiFixture"
import { expectDialogContained, expectNoHorizontalOverflow, readerRowButton } from "./support/readerAssertions"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer
test.beforeEach(async () => { server = await startProductionServer() })
test.afterEach(async () => { await server?.stop() })

test("launch keeps a readable status and a non-interactive responsive silhouette", async ({ page }, testInfo) => {
  let releaseBootstrap = () => {}
  const bootstrapGate = new Promise<void>((resolve) => { releaseBootstrap = resolve })
  await page.route("**/api/v1/bootstrap", async (route) => {
    await bootstrapGate
    await route.continue()
  })

  try {
    await page.goto(server.baseURL, { waitUntil: "domcontentloaded" })
    const loading = page.getByRole("region", { name: "Preparing Raindrop" })
    await expect(loading).toHaveAttribute("aria-busy", "true")
    await expect(loading.getByRole("status")).toContainText("Raindrop")
    await expect(loading.getByRole("button")).toHaveCount(0)
    await expect(loading.getByRole("link")).toHaveCount(0)
    await expectNoHorizontalOverflow(page)
    await page.screenshot({ path: testInfo.outputPath("launch-light.png"), animations: "disabled" })
    await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" })
    await expect(loading.locator(".raindrop-launch-indicator")).toHaveCSS("animation-name", "none")
    await page.screenshot({ path: testInfo.outputPath("launch-dark.png"), animations: "disabled" })
  } finally {
    releaseBootstrap()
  }

  await expect(page.getByRole("heading", { name: "Set up Raindrop" })).toBeVisible()
})

test("tactile surfaces preserve reading, keyboard feedback, and independent material preferences", async ({ page }, testInfo) => {
  testInfo.setTimeout(60_000)
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme: "light" })
  await installReaderApiFixture(page)
  await completeSetup(page, server, createCredentials())
  await readerRowButton(page, readerIds.firstEntry).click()
  await expect(page.getByRole("article")).toBeVisible()
  await expectNoHorizontalOverflow(page)
  await page.screenshot({ path: testInfo.outputPath("reader-light.png"), animations: "disabled" })
  if (testInfo.project.use.isMobile) {
    const readingTrigger = page.locator(".reader-reading-dock-trigger")
    const restingCenter = await readingTrigger.evaluate((element) => {
      const bounds = element.getBoundingClientRect()
      return bounds.left + bounds.width / 2
    })
    await readingTrigger.hover()
    await page.mouse.down()
    try {
      await readingTrigger.evaluate(async (element) => {
        await Promise.all(element.getAnimations().map((animation) => animation.finished.catch(() => {})))
      })
      const pressedCenter = await readingTrigger.evaluate((element) => {
        const bounds = element.getBoundingClientRect()
        return bounds.left + bounds.width / 2
      })
      expect(pressedCenter).toBeCloseTo(restingCenter, 0)
    } finally {
      await page.mouse.move(0, 0)
      await page.mouse.up()
    }
  }

  const dialog = await openAppearance(page)
  await expect(page.locator("html")).toHaveAttribute("data-raindrop-input", "pointer")
  await expectDialogContained(dialog, page)
  await dialog.getByRole("radio", { name: "Dark", exact: true }).click()
  await page.screenshot({ path: testInfo.outputPath("settings-dark-preview.png"), animations: "disabled" })

  const appearance = dialog.getByRole("button", { name: /^Appearance/u })
  await appearance.focus()
  await appearance.press("Enter")
  await expect(page.locator("html")).toHaveAttribute("data-raindrop-input", "keyboard")
  await expect(dialog.locator(".reader-panel-transition")).toHaveCSS("transition-duration", "0s")
  await expect(dialog).toHaveCSS("animation-name", "none")

  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark")
  await closeSources(page)
  await page.screenshot({ path: testInfo.outputPath("reader-dark.png"), animations: "disabled" })

  const appearanceDialog = await openAppearance(page)
  await appearanceDialog.getByRole("switch", { name: "Soft shadows" }).uncheck()
  const preview = appearanceDialog.locator(".reader-appearance-preview")
  await expect(preview).toHaveCSS("--surface-control-sheen", "none")
  await expect(preview).toHaveCSS("--shadow-control", "none")
  await expect(appearanceDialog.locator(".reader-preview-material")).toHaveCSS("backdrop-filter", "blur(20px) saturate(1.18)")
  await appearanceDialog.getByRole("switch", { name: "Frosted surfaces" }).uncheck()
  await expect(appearanceDialog.locator(".reader-preview-material")).toHaveCSS("backdrop-filter", "none")
  await expectNoHorizontalOverflow(page)

  await page.emulateMedia({ reducedMotion: "reduce" })
  const personal = appearanceDialog.getByRole("button", { name: /^Personal/u })
  await personal.click()
  await expect(appearanceDialog.locator(".reader-panel-transition")).toHaveCSS("transform", "none")
  await appearance.focus()
  await appearance.press("Enter")
  await expect(appearanceDialog.locator(".reader-panel-transition")).toHaveCSS("transition-duration", "0s")
  await expectDialogContained(appearanceDialog, page)
  expect(errors).toEqual([])
})

test("system accessibility preferences flatten translucent layers without hiding their controls", async ({ page }) => {
  await installReaderApiFixture(page)
  await completeSetup(page, server, createCredentials())
  const dialog = await openAppearance(page)
  const media = await page.context().newCDPSession(page)
  await media.send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-color-scheme", value: "dark" },
      { name: "prefers-reduced-transparency", value: "reduce" },
      { name: "prefers-contrast", value: "more" },
    ],
  })

  await expect.poll(() => dialog.evaluate((element) => getComputedStyle(element, "::backdrop").backdropFilter)).toBe("none")
  await expect(dialog).toHaveCSS("backdrop-filter", "none")
  await expect(dialog.locator(".reader-appearance-preview")).toHaveCSS("--surface-control-sheen", "none")
  await expect(dialog.getByRole("button", { name: "Save changes" })).toBeEnabled()
  await expectDialogContained(dialog, page)
  await expectNoHorizontalOverflow(page)
})

test("keyboard sources navigation never waits for a drawer transition", async ({ page }, testInfo) => {
  test.skip(!testInfo.project.use.isMobile, "The compact source drawer is exercised on touch layouts.")
  await installReaderApiFixture(page)
  await completeSetup(page, server, createCredentials())
  const openSources = page.getByRole("button", { name: "Open sources" })
  await openSources.focus()
  await openSources.press("Enter")
  const sources = page.locator("dialog.astryx-mobile-nav")
  await expect(sources).toBeVisible()
  await expect(sources.locator(":scope > div")).toHaveCSS("transition-duration", "0s")
  await page.keyboard.press("Escape")
  expect(await sources.evaluate((element) => (element as HTMLDialogElement).open)).toBe(false)
  await expect(openSources).toBeFocused()
})

test("disabling decorative shadows preserves category drop feedback", async ({ page }, testInfo) => {
  test.skip(Boolean(testInfo.project.use.isMobile), "Native feed drag targets are desktop-only.")
  await installReaderApiFixture(page)
  await completeSetup(page, server, createCredentials())
  const dialog = await openAppearance(page)
  await dialog.getByRole("switch", { name: "Soft shadows" }).uncheck()
  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expect(page.locator("html")).toHaveAttribute("data-raindrop-shadows", "false")

  const feed = page.locator(".reader-feed-drag-label").filter({ hasText: "Quiet Web" })
  const category = page.locator(".reader-category-drop-label").filter({ hasText: "Engineering" })
  const transfer = await page.evaluateHandle(() => new DataTransfer())
  try {
    await feed.dispatchEvent("dragstart", { dataTransfer: transfer })
    await category.dispatchEvent("dragover", { dataTransfer: transfer })
    await expect(category).toHaveAttribute("data-drop-target", "true")
    await expect(category).not.toHaveCSS("box-shadow", "none")
    expect(await category.evaluate((element) => getComputedStyle(element).boxShadow)).toContain("inset")
  } finally {
    await feed.dispatchEvent("dragend", { dataTransfer: transfer })
    await transfer.dispose()
  }
})

async function openAppearance(page: Page) {
  const menu = page.getByRole("button", { name: "Open menu" })
  if (!(await menu.isVisible())) await page.getByRole("button", { name: "Open sources" }).click()
  await menu.click()
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await dialog.getByRole("button", { name: /^Appearance/u }).click()
  return dialog
}

async function closeSources(page: Page) {
  const sources = page.getByRole("dialog", { name: "Sources", exact: true })
  if (await sources.isVisible()) {
    await page.keyboard.press("Escape")
    await expect(sources).not.toBeVisible()
  }
}
