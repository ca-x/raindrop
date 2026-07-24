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
  await chooseReadingDisplay(page)
  const trigger = page.getByRole("button", { name: "Open menu" })
  await trigger.click()
  await page.getByRole("menuitem", { name: "Settings" }).click()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await expect(dialog.getByText("Manage account, reading, plugins, and backups.")).toBeVisible()
  await verifySettingsNavigationStates(dialog, "Personal", "Reading")
  await verifyBackupFormAlignment(dialog)
  await choosePreferences(dialog, {
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: "COMPACT",
    readingFontScale: 120,
    readingFontFamily: "SANS",
    readingCustomFontId: null,
    readingColorScheme: "SEPIA",
    linkOpenMode: "CURRENT_TAB",
  })
  await dialog.locator('input[type="file"][accept*=".woff2"]').setInputFiles({
    name: "Editorial.woff2",
    mimeType: "font/woff2",
    buffer: Buffer.from("wOF2fixture-font"),
  })
  await dialog.getByRole("button", { name: "Upload font" }).click()
  await expect(dialog.getByText("Editorial", { exact: true })).toBeVisible()
  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expectPresentation(page, {
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: "COMPACT",
    readingFontScale: 120,
    readingFontFamily: "SANS",
    readingCustomFontId: null,
    readingColorScheme: "SEPIA",
    linkOpenMode: "CURRENT_TAB",
  })
  await expect(page.getByRole("button", { name: "打开菜单" })).toBeFocused()

  expect(fixture.preferences.patches).toHaveLength(6)
  expect(fixture.preferences.patches[5]).toMatchObject({
    body: {
      locale: "zh-CN",
      themeMode: "DARK",
      layoutDensity: "COMPACT",
    },
  })
  expect(fixture.preferences.patches.every((patch) => Boolean(patch.csrf))).toBe(true)

  const readingToolbar = page.getByRole("toolbar", { name: "正文显示控制" })
  await page.getByRole("article").getByRole("heading", { level: 1 }).focus()
  await page.mouse.move(0, 0)
  await expect(readingToolbar).toBeHidden()
  await page.locator(".reader-reading-dock").hover()
  await expect(readingToolbar).toBeVisible()
  await readingToolbar.getByRole("button", { name: "选择正文字体" }).click()
  const fontDialog = page.getByRole("dialog", { name: "选择正文字体" })
  await fontDialog.getByRole("button", { name: "Editorial" }).click()
  await expect.poll(() => fixture.preferences.current().readingCustomFontId).not.toBeNull()
  await page.getByRole("button", { name: "打开菜单" }).click()
  await page.getByRole("menuitem", { name: "设置" }).click()
  const reopenedSettings = page.getByRole("dialog", { name: "设置" })
  await expect(reopenedSettings.getByText("管理账户、阅读、插件与备份设置。")).toBeVisible()
  await verifySettingsNavigationStates(reopenedSettings, "个人", "阅读")
  await reopenedSettings.getByRole("button", { name: /^阅读(?:\s|$)/u }).click()
  await reopenedSettings.getByRole("button", { name: "删除字体“Editorial”" }).click()
  await expect(reopenedSettings.getByText("Editorial", { exact: true })).toHaveCount(0)
  await reopenedSettings.getByRole("button", { name: "取消" }).click()
  await expect.poll(() => fixture.preferences.current().readingCustomFontId).toBeNull()

  await page.reload({ waitUntil: "domcontentloaded" })
  await expect(page).toHaveURL(/\/reader\/unread(?:\/entry\/[^/]+)?$/u)
  await expect(page.getByRole("button", { name: "打开菜单" })).toBeVisible()
  await expectPresentation(page, fixture.preferences.current())
  expect(fixture.preferences.patches).toHaveLength(7)
  await expectNoHorizontalOverflow(page)
}

async function verifyBackupFormAlignment(dialog: Locator): Promise<void> {
  await dialog.getByRole("button", { name: /^Backup\b/u }).click()
  await dialog.getByRole("button", { name: "Add S3 target" }).click()
  await expectInputRowAligned(dialog, /^Name\b/u, /^HTTPS endpoint\b/u)

  const pathStyle = dialog.getByRole("switch", { name: /Use path-style addressing/u })
  await expect(pathStyle.locator("xpath=ancestor::*[contains(@class, 'astryx-switch-field')][1]"))
    .toHaveAttribute("data-label-spacing", "spread")

  await dialog.getByRole("button", { name: "Cancel" }).click()
  await dialog.getByRole("button", { name: "WebDAV" }).click()
  await dialog.getByRole("button", { name: "Add WEBDAV target" }).click()
  await expectInputRowAligned(dialog, /^Name\b/u, /^HTTPS endpoint\b/u)
  await dialog.getByRole("button", { name: "Cancel" }).click()
  await dialog.getByRole("button", { name: /^Personal\b/u }).click()
}

async function expectInputRowAligned(
  dialog: Locator,
  leftLabel: RegExp,
  rightLabel: RegExp,
): Promise<void> {
  const [left, right] = await Promise.all([
    dialog.getByLabel(leftLabel).locator("xpath=..").boundingBox(),
    dialog.getByLabel(rightLabel).locator("xpath=..").boundingBox(),
  ])
  expect(left).not.toBeNull()
  expect(right).not.toBeNull()
  expect(Math.abs(left!.y - right!.y)).toBeLessThanOrEqual(1)
  expect(Math.abs(left!.height - right!.height)).toBeLessThanOrEqual(1)
  expect(Math.abs(left!.width - right!.width)).toBeLessThanOrEqual(1)
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
    readingFontScale: 100,
    readingFontFamily: "SERIF",
    readingCustomFontId: null,
    readingColorScheme: "SEPIA",
    linkOpenMode: "CURRENT_TAB",
  }
  await choosePreferences(dialog, desired)

  if (failBeforeSuccess) {
    const failedPatchCount = fixture.preferences.patches.length + 1
    fixture.preferences.failNextPatch()
    const failedResponse = page.waitForResponse((response) =>
      response.request().method() === "PATCH" &&
      new URL(response.url()).pathname === "/api/v2/preferences" &&
      response.status() === 500
    )
    await Promise.all([
      failedResponse,
      dialog.getByRole("button", { name: "Save changes" }).click(),
    ])
    expect(fixture.preferences.patches).toHaveLength(failedPatchCount)
    await expect(dialog.getByText("Preferences could not be saved")).toBeVisible()
    await expect(dialog.getByRole("combobox", { name: "Article font" })).toHaveCount(0)
    await expect(dialog.getByRole("radio", { name: "Sepia" })).toBeChecked()
    await expect(dialog.getByRole("radio", { name: "Current page" })).toBeChecked()
    await expect(dialog.getByRole("slider", { name: "Reading size" })).toHaveCount(0)
    await dialog.getByRole("button", { name: "Personal" }).click()
    await expect(dialog.getByRole("radio", { name: "Dark" })).toBeChecked()
    await expect(dialog.getByRole("radio", { name: "中文" })).toBeChecked()
    await expect(dialog.getByRole("radio", { name: "Spacious" })).toBeChecked()
    await expectPresentation(page, {
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
      readingFontFamily: "SERIF",
      readingCustomFontId: null,
      readingColorScheme: "AUTO",
      linkOpenMode: "NEW_TAB",
    })
    expect(fixture.preferences.current()).toEqual({
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
      readingFontFamily: "SERIF",
      readingCustomFontId: null,
      readingColorScheme: "AUTO",
      linkOpenMode: "NEW_TAB",
    })
  }

  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog).not.toBeVisible()
  await expectPresentation(page, desired)
  const reopened = await openCompactSettings(page, "zh-CN")
  await expect(reopened.dialog.getByText(
    "管理账户、阅读、插件与备份设置。",
  )).toBeVisible()
  await expectDialogContained(reopened.dialog, page)
  await expectNoHorizontalOverflow(page)
  await reopened.dialog.getByRole("button", { name: "取消" }).click()
  await expect(reopened.sources).toBeVisible()
  await expect(reopened.sources.getByRole("button", { name: "打开菜单" })).toBeFocused()
}

async function verifySettingsNavigationStates(
  dialog: Locator,
  activeLabel: string,
  hoverLabel: string,
): Promise<void> {
  const active = dialog.getByRole("button", {
    name: new RegExp(`^${activeLabel}(?:\\s|$)`, "u"),
  })
  const hovered = dialog.getByRole("button", {
    name: new RegExp(`^${hoverLabel}(?:\\s|$)`, "u"),
  })
  await hovered.hover()
  const [activeStyle, hoverStyle] = await Promise.all([
    active.evaluate((element) => {
      const style = getComputedStyle(element)
      return { background: style.backgroundColor, shadow: style.boxShadow }
    }),
    hovered.evaluate((element) => {
      const style = getComputedStyle(element)
      return { background: style.backgroundColor, shadow: style.boxShadow }
    }),
  ])
  expect(activeStyle.background).not.toBe(hoverStyle.background)
  expect(activeStyle.shadow).not.toBe("none")
  expect(hoverStyle.shadow).not.toBe("none")
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
  await dialog.getByRole("button", { name: /^(?:Reading\b|阅读)/u }).click()
  await dialog.getByRole("radio", {
    name: {
      AUTO: /Auto|自动/u,
      PAPER: /Paper|纸白/u,
      SEPIA: /Sepia|米黄/u,
      GRAY: /Gray|灰色/u,
    }[preferences.readingColorScheme],
  }).click()
  await dialog.getByRole("radio", {
    name: preferences.linkOpenMode === "CURRENT_TAB"
      ? /Current page|当前页面/u
      : /New window|新窗口/u,
  }).click()
}

async function chooseReadingDisplay(page: Page): Promise<void> {
  const toolbar = page.getByRole("toolbar", { name: "Article display controls" })
  await page.getByRole("article").getByRole("heading", { level: 1 }).focus()
  await page.mouse.move(0, 0)
  await expect(toolbar).toBeHidden()
  await page.locator(".reader-reading-dock").hover()
  await expect(toolbar).toBeVisible()
  await toolbar.getByRole("button", { name: "Choose article font" }).click()
  const fontDialog = page.getByRole("dialog", { name: "Choose article font" })
  await fontDialog.getByRole("button", { name: "Sans serif" }).click()
  await page.locator(".reader-reading-dock").hover()
  const increase = toolbar.getByRole("button", { name: "Increase article text size" })
  for (let value = 100; value < 120; value += 5) await increase.click()
  await expect(
    toolbar.getByRole("button", { name: "Reset text size, currently 120%" }),
  ).toBeVisible()
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
    font: document.documentElement.dataset.raindropReadingFont,
    color: document.documentElement.dataset.raindropReadingColor,
  }))).toEqual({
    locale: preferences.locale,
    theme: preferences.themeMode === "SYSTEM" ? null : preferences.themeMode.toLowerCase(),
    density: preferences.layoutDensity.toLowerCase(),
    scale: `${preferences.readingFontScale}%`,
    font: preferences.readingFontFamily.toLowerCase(),
    color: preferences.readingColorScheme.toLowerCase(),
  })
}

function themeLabel(theme: UserPreferences["themeMode"]): string {
  return { SYSTEM: "System", LIGHT: "Light", DARK: "Dark" }[theme]
}

function densityLabel(density: UserPreferences["layoutDensity"]): string {
  return { COMPACT: "Compact", BALANCED: "Balanced", SPACIOUS: "Spacious" }[density]
}
