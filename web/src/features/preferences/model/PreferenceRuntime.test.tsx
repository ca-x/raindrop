import { readFileSync } from "node:fs"
import { join } from "node:path"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale, i18n } from "../../../shared/i18n/i18n"
import type { UserPreferences } from "../api/preferences.generated"
import {
  PREFERENCE_HINT_KEY,
  writePreferenceHint,
} from "./preferenceCache"
import { usePreferenceRuntime } from "./PreferenceRuntime"

const appliedPreferences: UserPreferences = {
  locale: "zh-CN",
  themeMode: "DARK",
  layoutDensity: "COMPACT",
  readingFontScale: 130,
  readingFontFamily: "SANS",
  readingColorScheme: "SEPIA",
  linkOpenMode: "CURRENT_TAB",
}

beforeEach(() => {
  localStorage.clear()
  document.documentElement.removeAttribute("data-theme")
  document.documentElement.removeAttribute("data-raindrop-density")
  document.documentElement.removeAttribute("data-raindrop-reading-font")
  document.documentElement.removeAttribute("data-raindrop-reading-color")
  document.documentElement.style.removeProperty("--raindrop-reading-scale")
  Object.defineProperty(navigator, "language", {
    configurable: true,
    value: "en-US",
  })
  activateLocale("en")
})

describe("PreferenceRuntime", () => {
  it("starts from stable browser defaults when no hint exists", async () => {
    renderRuntime()

    expect(readProbe()).toEqual({
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
      readingFontFamily: "SERIF",
      readingColorScheme: "AUTO",
      linkOpenMode: "NEW_TAB",
    })
    await expectPresentation({
      lang: "en",
      theme: null,
      density: "balanced",
      scale: "100%",
      font: "serif",
      color: "auto",
    })
  })

  it("preserves an already activated browser-local locale when no hint exists", () => {
    activateLocale("zh-CN")
    renderRuntime()

    expect(readProbe().locale).toBe("zh-CN")
  })

  it("hydrates the ASTRYX theme, locale, density, and reading scale from a hint", async () => {
    writePreferenceHint(appliedPreferences)
    renderRuntime()

    expect(readProbe()).toEqual(appliedPreferences)
    await expectPresentation({
      lang: "zh-CN",
      theme: "dark",
      density: "compact",
      scale: "130%",
      font: "sans",
      color: "sepia",
    })
    expect(i18n.locale).toBe("zh-CN")
  })

  it("applies one complete preference state and replaces the validated hint", async () => {
    renderRuntime()
    fireEvent.click(screen.getByRole("button", { name: "apply preferences" }))

    await waitFor(() => expect(readProbe()).toEqual(appliedPreferences))
    await expectPresentation({
      lang: "zh-CN",
      theme: "dark",
      density: "compact",
      scale: "130%",
      font: "sans",
      color: "sepia",
    })
    expect(JSON.parse(localStorage.getItem(PREFERENCE_HINT_KEY) ?? "null")).toEqual({
      schemaVersion: 2,
      preferences: appliedPreferences,
    })
  })

  it("clears only the persisted hint", async () => {
    writePreferenceHint(appliedPreferences)
    renderRuntime()
    fireEvent.click(screen.getByRole("button", { name: "clear preference hint" }))

    expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
    expect(readProbe()).toEqual(appliedPreferences)
  })

  it("fails fast when the runtime hook is used outside its provider", () => {
    expect(() => render(<RuntimeProbe />)).toThrow(
      "usePreferenceRuntime must be used within PreferenceRuntimeProvider",
    )
  })
})

describe("theme-bootstrap.js", () => {
  const source = () =>
    readFileSync(join(process.cwd(), "public/theme-bootstrap.js"), "utf8")

  it("independently validates and applies the exact presentation hint", () => {
    writePreferenceHint(appliedPreferences)
    window.eval(source())

    expect(document.documentElement.getAttribute("data-theme")).toBe("dark")
    expect(document.documentElement.lang).toBe("zh-CN")
    expect(document.documentElement.dataset.raindropDensity).toBe("compact")
    expect(
      document.documentElement.style.getPropertyValue("--raindrop-reading-scale"),
    ).toBe("130%")
    expect(document.documentElement.dataset.raindropReadingFont).toBe("sans")
    expect(document.documentElement.dataset.raindropReadingColor).toBe("sepia")
  })

  it("removes a malformed hint without applying any of its values", () => {
    localStorage.setItem(
      PREFERENCE_HINT_KEY,
      JSON.stringify({
        schemaVersion: 2,
        preferences: { ...appliedPreferences, csrfToken: "must-not-persist" },
      }),
    )
    window.eval(source())

    expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
    expect(document.documentElement.getAttribute("data-theme")).toBeNull()
    expect(document.documentElement.dataset.raindropDensity).toBeUndefined()
    expect(document.documentElement.dataset.raindropReadingFont).toBeUndefined()
    expect(document.documentElement.dataset.raindropReadingColor).toBeUndefined()
    expect(
      document.documentElement.style.getPropertyValue("--raindrop-reading-scale"),
    ).toBe("")
  })
})

function renderRuntime() {
  render(
    <Providers>
      <RuntimeProbe />
    </Providers>,
  )
}

function RuntimeProbe() {
  const { preferences, apply, clearHint } = usePreferenceRuntime()
  return (
    <>
      <output data-testid="preferences">{JSON.stringify(preferences)}</output>
      <button type="button" onClick={() => apply(appliedPreferences)}>
        apply preferences
      </button>
      <button type="button" onClick={clearHint}>
        clear preference hint
      </button>
    </>
  )
}

function readProbe(): UserPreferences {
  return JSON.parse(screen.getByTestId("preferences").textContent ?? "null")
}

async function expectPresentation(expected: {
  lang: string
  theme: string | null
  density: string
  scale: string
  font: string
  color: string
}) {
  await waitFor(() => {
    expect(document.documentElement.lang).toBe(expected.lang)
    expect(document.documentElement.getAttribute("data-theme")).toBe(expected.theme)
    expect(document.documentElement.dataset.raindropDensity).toBe(expected.density)
    expect(
      document.documentElement.style.getPropertyValue("--raindrop-reading-scale"),
    ).toBe(expected.scale)
    expect(document.documentElement.dataset.raindropReadingFont).toBe(expected.font)
    expect(document.documentElement.dataset.raindropReadingColor).toBe(expected.color)
  })
}
