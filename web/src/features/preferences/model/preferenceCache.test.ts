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
}

describe("preference presentation hint", () => {
  beforeEach(() => localStorage.clear())

  it("stores and reads only the exact versioned non-sensitive shape", () => {
    writePreferenceHint(preferences)

    expect(JSON.parse(localStorage.getItem(PREFERENCE_HINT_KEY) ?? "null")).toEqual({
      schemaVersion: 1,
      preferences,
    })
    expect(readPreferenceHint()).toEqual(preferences)
    expect(localStorage.length).toBe(1)
  })

  it.each([
    ["invalid JSON", "{"],
    ["array", JSON.stringify([])],
    ["missing version", JSON.stringify({ preferences })],
    ["wrong version", JSON.stringify({ schemaVersion: 2, preferences })],
    [
      "unknown top-level field",
      JSON.stringify({ schemaVersion: 1, preferences, csrfToken: "must-not-persist" }),
    ],
    [
      "unknown preference field",
      JSON.stringify({
        schemaVersion: 1,
        preferences: { ...preferences, userId: "must-not-persist" },
      }),
    ],
    [
      "missing preference field",
      JSON.stringify({
        schemaVersion: 1,
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
        schemaVersion: 1,
        preferences: { ...preferences, readingFontScale: 131 },
      }),
    ],
  ])("rejects and removes a malformed %s hint", (_name, stored) => {
    localStorage.setItem(PREFERENCE_HINT_KEY, stored)

    expect(readPreferenceHint()).toBeNull()
    expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
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

  it("clears the presentation hint", () => {
    writePreferenceHint(preferences)
    clearPreferenceHint()
    expect(localStorage.getItem(PREFERENCE_HINT_KEY)).toBeNull()
  })
})
