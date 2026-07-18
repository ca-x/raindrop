import { expect, type Locator, type Page } from "@playwright/test"

import type { UserPreferences } from "../../src/features/preferences/api/preferences.generated"
import type { ReaderApiFixture } from "./readerApiFixture"
import {
  expectDialogContained,
  expectNoHorizontalOverflow,
} from "./readerAssertions"

export async function verifyWidePreferences(
  page: Page,
  fixture: ReaderApiFixture,
): Promise<void> {
  const trigger = page.getByRole("button", { name: "Open menu" })
  await trigger.click()
  await page.getByRole("menuitem", { name: "Settings" }).click()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await choosePreferences(dialog, {
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: "COMPACT",
    readingFontScale: 120,
  })
  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expectPresentation(page, {
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: "COMPACT",
    readingFontScale: 120,
  })
  await expect(page.getByRole("button", { name: "打开菜单" })).toBeFocused()

  expect(fixture.preferences.patches).toHaveLength(1)
  expect(fixture.preferences.patches[0]).toMatchObject({
    body: {
      locale: "zh-CN",
      themeMode: "DARK",
      layoutDensity: "COMPACT",
      readingFontScale: 120,
    },
  })
  expect(fixture.preferences.patches[0]?.csrf).toBeTruthy()

  await page.reload({ waitUntil: "domcontentloaded" })
  await expect(page).toHaveURL(/\/reader\/unread(?:\/entry\/[^/]+)?$/u)
  await expect(page.getByRole("button", { name: "打开菜单" })).toBeVisible()
  await expectPresentation(page, fixture.preferences.current())
  expect(fixture.preferences.patches).toHaveLength(1)
  await expectNoHorizontalOverflow(page)
}

export async function verifyMediumPreferencesFocus(page: Page): Promise<void> {
  const openSources = page.getByRole("button", { name: "Open sources" })
  const sources = page.getByRole("dialog", { name: "Sources" })
  if (!(await sources.isVisible())) await openSources.click()
  const trigger = sources.getByRole("button", { name: "Open menu" })
  await trigger.click()
  await page.getByRole("menuitem", { name: "Settings" }).click()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await expect(dialog).toBeVisible()
  await expect(sources).not.toBeVisible()
  await dialog.getByRole("button", { name: "Cancel" }).click()
  await expect(sources).toBeVisible()
  await expect(sources.getByRole("button", { name: "Open menu" })).toBeFocused()
  await expectNoHorizontalOverflow(page)
}

export async function verifyCompactPreferences(
  page: Page,
  fixture: ReaderApiFixture,
  failBeforeSuccess: boolean,
): Promise<void> {
  const { dialog } = await openCompactSettings(page, "en")
  await expectDialogContained(dialog, page)
  await expectNoHorizontalOverflow(page)
  const desired: UserPreferences = {
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: failBeforeSuccess ? "SPACIOUS" : "COMPACT",
    readingFontScale: 120,
  }
  await choosePreferences(dialog, desired)

  if (failBeforeSuccess) {
    const failedPatchCount = fixture.preferences.patches.length + 1
    fixture.preferences.failNextPatch()
    const failedResponse = page.waitForResponse((response) =>
      response.request().method() === "PATCH" &&
      new URL(response.url()).pathname === "/api/v1/preferences" &&
      response.status() === 500
    )
    await Promise.all([
      failedResponse,
      dialog.getByRole("button", { name: "Save changes" }).click(),
    ])
    expect(fixture.preferences.patches).toHaveLength(failedPatchCount)
    await expect(dialog.getByText("Preferences could not be saved")).toBeVisible()
    await expect(dialog.getByRole("radio", { name: "Dark" })).toBeChecked()
    await expect(dialog.getByRole("radio", { name: "中文" })).toBeChecked()
    await expect(dialog.getByRole("radio", { name: "Spacious" })).toBeChecked()
    await expect(dialog.getByRole("radio", { name: "120%" })).toBeChecked()
    await expectPresentation(page, {
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
    })
    expect(fixture.preferences.current()).toEqual({
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
    })
  }

  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expectPresentation(page, desired)
  const reopened = await openCompactSettings(page, "zh-CN")
  await expect(reopened.dialog.getByText(
    "调整 Raindrop 的显示方式，不打断当前阅读。",
  )).toBeVisible()
  await expectDialogContained(reopened.dialog, page)
  await expectNoHorizontalOverflow(page)
  await reopened.dialog.getByRole("button", { name: "取消" }).click()
  await expect(reopened.sources).toBeVisible()
  await expect(reopened.sources.getByRole("button", { name: "打开菜单" })).toBeFocused()
}

async function choosePreferences(
  dialog: Locator,
  preferences: UserPreferences,
): Promise<void> {
  await dialog.getByRole("radio", { name: themeLabel(preferences.themeMode) }).click()
  await dialog.getByRole("radio", {
    name: preferences.locale === "zh-CN" ? "中文" : "English",
  }).click()
  await dialog.getByRole("radio", {
    name: densityLabel(preferences.layoutDensity),
  }).click()
  await dialog.getByRole("radio", {
    name: `${preferences.readingFontScale}%`,
  }).click()
}

async function openCompactSettings(
  page: Page,
  locale: UserPreferences["locale"],
): Promise<{ dialog: Locator; sources: Locator }> {
  const labels = locale === "zh-CN"
    ? { sources: "来源", openSources: "打开来源", menu: "打开菜单", settings: "设置" }
    : { sources: "Sources", openSources: "Open sources", menu: "Open menu", settings: "Settings" }
  const sources = page.getByRole("dialog", { name: labels.sources })
  if (!(await sources.isVisible())) {
    await page.getByRole("button", { name: labels.openSources }).click()
  }
  await sources.getByRole("button", { name: labels.menu }).click()
  await page.getByRole("menuitem", { name: labels.settings }).click()
  return {
    dialog: page.getByRole("dialog", { name: labels.settings }),
    sources,
  }
}

async function expectPresentation(
  page: Page,
  preferences: UserPreferences,
): Promise<void> {
  await expect.poll(() => page.evaluate(() => ({
    locale: document.documentElement.lang,
    theme: document.documentElement.getAttribute("data-theme"),
    density: document.documentElement.dataset.raindropDensity,
    scale: document.documentElement.style.getPropertyValue("--raindrop-reading-scale"),
  }))).toEqual({
    locale: preferences.locale,
    theme: preferences.themeMode === "SYSTEM" ? null : preferences.themeMode.toLowerCase(),
    density: preferences.layoutDensity.toLowerCase(),
    scale: `${preferences.readingFontScale}%`,
  })
}

function themeLabel(theme: UserPreferences["themeMode"]): string {
  return { SYSTEM: "System", LIGHT: "Light", DARK: "Dark" }[theme]
}

function densityLabel(density: UserPreferences["layoutDensity"]): string {
  return { COMPACT: "Compact", BALANCED: "Balanced", SPACIOUS: "Spacious" }[density]
}
