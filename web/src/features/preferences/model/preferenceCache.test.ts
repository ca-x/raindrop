import { beforeEach, describe, expect, it, vi } from "vitest"

import type { UserPreferences } from "../api/preferences.generated"
import {
  PREFERENCE_HINT_KEY,
  clearPreferenceHint,
  readPreferenceHint,
  writePreferenceHint,
} from "./preferenceCache"

const preferences: UserPreferences = {
  locale: "zh-CN",
  themeMode: "DARK",
  layoutDensity: "COMPACT",
  readingFontScale: 110,
  readingFontFamily: "SANS",
  readingCustomFontId: null,
  readingColorScheme: "SEPIA",
  interfaceBlur: true,
  interfaceShadows: true,
  linkOpenMode: "CURRENT_TAB",
}

describe("preference presentation hint", () => {
  beforeEach(() => localStorage.clear())

  it("stores and reads only the exact versioned non-sensitive shape", () => {
    writePreferenceHint(preferences)

    expect(JSON.parse(localStorage.getItem(PREFERENCE_HINT_KEY) ?? "null")).toEqual({
      schemaVersion: 4,
      preferences,
    })
    expect(readPreferenceHint()).toEqual(preferences)
    expect(localStorage.length).toBe(1)
  })

  it.each([
    ["invalid JSON", "{"],
    ["array", JSON.stringify([])],
    ["missing version", JSON.stringify({ preferences })],
    ["wrong version", JSON.stringify({ schemaVersion: 5, preferences })],
    [
      "unknown top-level field",
      JSON.stringify({ schemaVersion: 2, preferences, csrfToken: "must-not-persist" }),
    ],
    [
      "unknown preference field",
      JSON.stringify({
        schemaVersion: 2,
        preferences: { ...preferences, userId: "must-not-persist" },
      }),
    ],
    [
      "missing preference field",
      JSON.stringify({
        schemaVersion: 2,
        preferences: {
          locale: "zh-CN",
          themeMode: "DARK",
          layoutDensity: "COMPACT",
        },
      }),
    ],
    [
      "invalid preference value",
      JSON.stringify({
        schemaVersion: 2,
        preferences: { ...preferences, readingFontScale: 131 },
      }),
    ],
  ])("rejects and removes a malformed %s hint", (_name, stored) => {
    localStorage.setItem(PREFERENCE_HINT_KEY, stored)

    expect(readPreferenceHint()).toBeNull()
    expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
  })

  it("migrates the v1 presentation hint with safe reading defaults", () => {
    localStorage.setItem(
      PREFERENCE_HINT_KEY,
      JSON.stringify({
        schemaVersion: 1,
        preferences: {
          locale: "zh-CN",
          themeMode: "DARK",
          layoutDensity: "COMPACT",
          readingFontScale: 110,
        },
      }),
    )

    expect(readPreferenceHint()).toEqual({
      locale: "zh-CN",
      themeMode: "DARK",
      layoutDensity: "COMPACT",
      readingFontScale: 110,
      readingFontFamily: "SERIF",
      readingCustomFontId: null,
      readingColorScheme: "AUTO",
      interfaceBlur: true,
      interfaceShadows: true,
      linkOpenMode: "NEW_TAB",
    })
    expect(JSON.parse(localStorage.getItem(PREFERENCE_HINT_KEY) ?? "null")).toEqual({
      schemaVersion: 4,
      preferences: expect.objectContaining({ readingFontFamily: "SERIF" }),
    })
  })

  it("keeps runtime behavior available when storage access fails", () => {
    vi.spyOn(localStorage, "getItem").mockImplementationOnce(() => {
      throw new DOMException("blocked", "SecurityError")
    })
    expect(readPreferenceHint()).toBeNull()

    vi.spyOn(localStorage, "setItem").mockImplementationOnce(() => {
      throw new DOMException("full", "QuotaExceededError")
    })
    expect(() => writePreferenceHint(preferences)).not.toThrow()
  })

  it.each([2, 3])("migrates v%s without discarding reading or theme choices", (schemaVersion) => {
    const { interfaceBlur: _blur, interfaceShadows: _shadows, ...previous } = preferences
    const legacy = schemaVersion === 2
      ? Object.fromEntries(Object.entries(previous).filter(([key]) => key !== "readingCustomFontId"))
      : previous
    localStorage.setItem(PREFERENCE_HINT_KEY, JSON.stringify({ schemaVersion, preferences: legacy }))
    expect(readPreferenceHint()).toEqual(preferences)
    expect(JSON.parse(localStorage.getItem(PREFERENCE_HINT_KEY) ?? "null").schemaVersion).toBe(4)
  })

  it("preserves independently disabled effects and rejects incorrectly typed flags", () => {
    const disabled = { ...preferences, interfaceBlur: false, interfaceShadows: true }
    writePreferenceHint(disabled)
    expect(readPreferenceHint()).toEqual(disabled)
    localStorage.setItem(PREFERENCE_HINT_KEY, JSON.stringify({
      schemaVersion: 4, preferences: { ...preferences, interfaceShadows: null },
    }))
    expect(readPreferenceHint()).toBeNull()
  })

  it("clears the presentation hint", () => {
    writePreferenceHint(preferences)
    clearPreferenceHint()
    expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
  })
})
