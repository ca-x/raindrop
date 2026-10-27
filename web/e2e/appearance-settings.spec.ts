import { expect, test, type Page } from "@playwright/test"

import { completeSetup, createCredentials } from "./support/app"
import { defaultPreferences } from "../src/features/preferences/model/preferenceTypes"
import { installReaderApiFixture, readerIds } from "./support/readerApiFixture"
import { expectDialogContained, expectNoHorizontalOverflow, readerRowButton } from "./support/readerAssertions"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer
test.beforeAll(async () => { server = await startProductionServer() })
test.afterAll(async () => { await server?.stop() })

for (const schemaVersion of [3, 4]) {
  test(`first paint restores schema ${schemaVersion} before React loads`, async ({ page }) => {
    const preferences: Record<string, unknown> = {
      ...defaultPreferences("en"),
      themeMode: "DARK",
      interfaceBlur: false,
      interfaceShadows: false,
      layoutDensity: "COMPACT",
      readingColorScheme: "PAPER",
    }
    if (schemaVersion === 3) {
      delete preferences.interfaceBlur
      delete preferences.interfaceShadows
    }
    await page.addInitScript((hint) => {
      localStorage.setItem("raindrop.preferences.v1", JSON.stringify(hint))
    }, { schemaVersion, preferences })
    await page.route(/\/assets\/index-[^/]+\.js$/u, (route) => route.fulfill({ contentType: "text/javascript", body: "" }))
    await page.goto(server.baseURL)
    const root = page.locator("html")
    await expect(page.locator("#root")).toBeEmpty()
    await expect(root).toHaveAttribute("data-theme", "dark")
    await expect(root).toHaveAttribute("data-astryx-theme", "neutral")
    await expect(page.locator("body")).toHaveCSS("background-color", "oklch(0.17 0.01 70)")
    await expect(root).toHaveAttribute("data-raindrop-blur", String(schemaVersion === 3))
    await expect(root).toHaveAttribute("data-raindrop-shadows", String(schemaVersion === 3))
    await expect(root).toHaveAttribute("data-raindrop-reading-color", "paper")
    await expect(root).toHaveAttribute("data-raindrop-density", "compact")
    await expect(root).toHaveAttribute("lang", "en")
  })
}

test("appearance, independent materials, and paper reading persist with the real preferences API", async ({ page }, testInfo) => {
  testInfo.setTimeout(90_000)
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.emulateMedia({ colorScheme: "light" })
  await installReaderApiFixture(page)
  await page.unroute("**/api/v2/preferences")
  await completeSetup(page, server, createCredentials())

  let dialog = await openAppearance(page)
  for (const item of await dialog.locator(".reader-settings-nav-item").all()) {
    const bounds = await item.evaluate((element) => ({
      item: element.getBoundingClientRect().toJSON(),
      copy: element.querySelector(".reader-settings-nav-copy")!.getBoundingClientRect().toJSON(),
    }))
    expect(bounds.copy.top).toBeGreaterThanOrEqual(bounds.item.top)
    expect(bounds.copy.bottom).toBeLessThanOrEqual(bounds.item.bottom)
  }
  const selected = dialog.getByRole("radio", { name: "System", exact: true })
  await selected.focus()
  await selected.press("Home")
  await expect(dialog.getByRole("radio", { name: "Light", exact: true })).toBeChecked()
  const preview = dialog.locator(".reader-appearance-preview")
  const lightPreviewBackground = await preview.evaluate((element) => getComputedStyle(element).backgroundColor)
  for (const option of await dialog.locator(".reader-theme-options [role=radio]").all()) {
    const bounds = await option.evaluate((element) => ({
      card: element.getBoundingClientRect().toJSON(),
      thumbnail: element.querySelector(".reader-theme-thumbnail")!.getBoundingClientRect().toJSON(),
    }))
    expect(bounds.card.height).toBeGreaterThanOrEqual(100)
    expect(bounds.thumbnail.top).toBeGreaterThan(bounds.card.top)
    expect(bounds.thumbnail.bottom).toBeLessThan(bounds.card.bottom)
  }
  await page.keyboard.press("ArrowRight")
  await expect(dialog.getByRole("radio", { name: "Dark", exact: true })).toBeChecked()
  await expect(preview).toHaveCSS("color-scheme", "dark")
  await expect.poll(() => preview.evaluate((element) => getComputedStyle(element).backgroundColor)).not.toBe(lightPreviewBackground)
  await expect(page.locator("html")).not.toHaveAttribute("data-theme", "dark")
  await dialog.getByRole("button", { name: /^Reading/u }).click()
  await dialog.getByRole("radio", { name: "Paper", exact: true }).click()
  await expect(dialog.locator(".reader-reading-sample")).toHaveCSS("background-color", "rgb(252, 250, 245)")
  const appearanceTab = dialog.getByRole("button", { name: /^Appearance/u })
  await appearanceTab.focus()
  await appearanceTab.press("Enter")
  await expect(dialog.locator(".reader-panel-transition")).toHaveCSS("transition-duration", "0s")
  await expect(dialog.locator(".reader-preview-title")).toHaveCSS("color", "rgb(36, 34, 31)")
  await dialog.locator(".reader-appearance-preview").scrollIntoViewIfNeeded()
  await page.screenshot({ path: testInfo.outputPath("appearance-dark-paper.png") })
  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark")

  for (const [blur, shadows] of [[false, true], [true, false], [false, false], [true, true]]) {
    dialog = await openAppearance(page)
    await dialog.getByRole("switch", { name: "Frosted surfaces" }).setChecked(blur)
    await dialog.getByRole("switch", { name: "Soft shadows" }).setChecked(shadows)
    const material = dialog.locator(".reader-preview-material")
    await expect(material).toHaveCSS("backdrop-filter", blur ? "blur(20px) saturate(1.18)" : "none")
    expect(await material.evaluate((element) => getComputedStyle(element).getPropertyValue("--shadow-color").trim() === "transparent")).toBe(!shadows)
    await expect(dialog.locator(".reader-reading-sample")).toHaveCSS("background-color", "rgb(252, 250, 245)")
    await expectDialogContained(dialog, page)
    await expectNoHorizontalOverflow(page)
    await dialog.getByRole("button", { name: "Save changes" }).click()
    await expect(dialog).not.toBeVisible()
    await expect(page.locator("html")).toHaveAttribute("data-raindrop-blur", String(blur))
    await expect(page.locator("html")).toHaveAttribute("data-raindrop-shadows", String(shadows))
    await page.reload({ waitUntil: "domcontentloaded" })
    await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
    await expect(page.locator("html")).toHaveAttribute("data-raindrop-blur", String(blur))
    await expect(page.locator("html")).toHaveAttribute("data-raindrop-shadows", String(shadows))
    const response = await page.request.get(`${server.baseURL}/api/v2/preferences`)
    expect(response.ok()).toBe(true)
    expect(await response.json()).toMatchObject({ themeMode: "DARK", interfaceBlur: blur, interfaceShadows: shadows, readingColorScheme: "PAPER" })
  }

  dialog = await openAppearance(page)
  await dialog.getByRole("switch", { name: "Frosted surfaces" }).uncheck()
  await dialog.getByRole("radio", { name: "Light", exact: true }).click()
  await dialog.getByRole("button", { name: "Cancel" }).click()
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark")
  await expect(page.locator("html")).toHaveAttribute("data-raindrop-blur", "true")

  await page.emulateMedia({ reducedMotion: "reduce" })
  dialog = await openAppearance(page)
  await dialog.getByRole("button", { name: /^Appearance/u }).focus()
  await dialog.getByRole("button", { name: /^Appearance/u }).press("Enter")
  await expect(dialog.locator(".reader-panel-transition")).toHaveCSS("transition-duration", "0s")
  await expect(dialog.getByRole("radio", { name: "Dark", exact: true })).toHaveCSS("transition-duration", "0s")
  await dialog.getByRole("radio", { name: "System", exact: true }).click()
  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expect(page.locator("html")).not.toHaveAttribute("data-theme")
  await page.emulateMedia({ colorScheme: "light" })
  const lightBackground = await page.locator("body").evaluate((element) => getComputedStyle(element).backgroundColor)
  await page.emulateMedia({ colorScheme: "dark" })
  await expect.poll(() => page.locator("body").evaluate((element) => getComputedStyle(element).backgroundColor)).not.toBe(lightBackground)

  const sources = page.getByRole("dialog", { name: "Sources", exact: true })
  if (await sources.isVisible()) {
    await page.keyboard.press("Escape")
    await expect(sources).not.toBeVisible()
  }
  await readerRowButton(page, readerIds.firstEntry).click()
  await expect(page.locator(".reader-article-plane")).toHaveCSS("background-color", "rgb(252, 250, 245)")
  await expect(page.locator(".reader-article-plane")).toHaveCSS("filter", "none")
  await expectNoHorizontalOverflow(page)
  expect(errors).toEqual([])
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

test("settings preserve navigation, aligned controls, and invalid-field focus on narrow screens", async ({ page }, testInfo) => {
  testInfo.setTimeout(60_000)
  const qualityServer = await startProductionServer()
  try {
    await installReaderApiFixture(page)
    await completeSetup(page, qualityServer, createCredentials())
    await page.setViewportSize({ width: 1280, height: 800 })
    await expect(page.getByRole("button", { name: "Open menu" })).toBeVisible()
    const dialog = await openAppearance(page)
    await dialog.getByRole("button", { name: /^Reading/u }).click()
    await page.setViewportSize({ width: 375, height: 812 })
    const navigation = dialog.getByRole("navigation", { name: "Settings sections" })
    const current = navigation.locator('[aria-current="page"]')
    await expect(current).toBeInViewport()
    await expect.poll(() => current.evaluate((element) => {
      const bounds = element.getBoundingClientRect()
      const parent = element.parentElement!.getBoundingClientRect()
      return {
        leftClipped: Math.max(0, Math.round(parent.left - bounds.left)),
        rightClipped: Math.max(0, Math.round(bounds.right - parent.right)),
      }
    })).toEqual({ leftClipped: 0, rightClipped: 0 })
    const size = dialog.getByRole("spinbutton", { name: "Reading size" })
    const reset = dialog.getByRole("button", { name: "Reset to default" })
    await size.fill("115")
    await size.press("Tab")
    await expect(reset).toBeEnabled()
    const inputBounds = await size.locator("..").boundingBox()
    const resetBounds = await reset.boundingBox()
    expect(inputBounds).not.toBeNull()
    expect(resetBounds).not.toBeNull()
    expect(Math.abs(inputBounds!.y + inputBounds!.height - resetBounds!.y - resetBounds!.height)).toBeLessThanOrEqual(1)
    await reset.click()
    await expect(size).toHaveValue("100")
    await expect(reset).toBeDisabled()
    const fileChooser = page.waitForEvent("filechooser")
    const fontPicker = dialog.getByRole("button", { name: "WOFF2, TTF, or OTF font file" })
    await fontPicker.focus()
    await fontPicker.press("Enter")
    await (await fileChooser).setFiles([])
    await expect(dialog.getByRole("button", { name: "Upload font" })).toBeDisabled()

    await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" })
    await dialog.getByRole("button", { name: /^Personal/u }).click()
    const nickname = dialog.getByRole("textbox", { name: /^Nickname/u })
    await nickname.fill("A".repeat(81))
    await dialog.getByRole("button", { name: "Save changes" }).click()
    await expect(nickname).toBeFocused()
    await expect(nickname).toHaveAttribute("aria-invalid", "true")
    await expect(nickname).toHaveValue("A".repeat(81))
    const footer = dialog.getByRole("button", { name: "Save changes" })
    await expect(footer).toBeInViewport()
    const bottomGap = await dialog.evaluate((element) => {
      const footer = element.querySelector(".reader-settings-footer")!
      return element.getBoundingClientRect().bottom - footer.getBoundingClientRect().bottom
    })
    expect(bottomGap).toBeLessThanOrEqual(2)
    await expectDialogContained(dialog, page)
    await expectNoHorizontalOverflow(page)
    await page.setViewportSize({ width: 812, height: 375 })
    await expect(footer).toBeInViewport()
    await expectDialogContained(dialog, page)
    await expectNoHorizontalOverflow(page)
    await page.screenshot({ path: testInfo.outputPath("settings-landscape-validation.png") })
  } finally {
    await qualityServer.stop()
  }
})
