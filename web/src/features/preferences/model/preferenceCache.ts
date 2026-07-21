import {
  isUserPreferences,
  type UserPreferences,
} from "../api/preferences.generated"

export const PREFERENCE_HINT_KEY = "raindrop.preferences.v1"

interface PreferenceHintV3 {
  schemaVersion: 3
  preferences: UserPreferences
}

export function readPreferenceHint(): UserPreferences | null {
  let stored: string | null
  try {
    stored = localStorage.getItem(PREFERENCE_HINT_KEY)
  } catch {
    return null
  }
  if (stored === null) return null

  try {
    const parsed: unknown = JSON.parse(stored)
    if (isPreferenceHint(parsed)) return parsed.preferences
    const migrated = migrateLegacyHint(parsed)
    if (migrated) {
      writePreferenceHint(migrated)
      return migrated
    }
    clearPreferenceHint()
    return null
  } catch {
    clearPreferenceHint()
    return null
  }
}

export function writePreferenceHint(preferences: UserPreferences): void {
  if (!isUserPreferences(preferences)) return
  const hint: PreferenceHintV3 = { schemaVersion: 3, preferences }
  try {
    localStorage.setItem(PREFERENCE_HINT_KEY, JSON.stringify(hint))
  } catch {
    // Presentation remains available even when storage is blocked or full.
  }
}

export function clearPreferenceHint(): void {
  try {
    localStorage.removeItem(PREFERENCE_HINT_KEY)
  } catch {
    // Storage is an optional presentation cache, never an application authority.
  }
}

function isPreferenceHint(value: unknown): value is PreferenceHintV3 {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["schemaVersion", "preferences"]) &&
    value.schemaVersion === 3 &&
    isUserPreferences(value.preferences)
  )
}

function migrateLegacyHint(value: unknown): UserPreferences | null {
  if (
    isRecord(value) &&
    hasOnlyKeys(value, ["schemaVersion", "preferences"]) &&
    value.schemaVersion === 2 &&
    isRecord(value.preferences) &&
    hasOnlyKeys(value.preferences, [
      "locale",
      "themeMode",
      "layoutDensity",
      "readingFontScale",
      "readingFontFamily",
      "readingColorScheme",
      "linkOpenMode",
    ])
  ) {
    const candidate = { ...value.preferences, readingCustomFontId: null }
    return isUserPreferences(candidate) ? candidate : null
  }
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, ["schemaVersion", "preferences"]) ||
    value.schemaVersion !== 1 ||
    !isRecord(value.preferences) ||
    !hasOnlyKeys(value.preferences, [
      "locale",
      "themeMode",
      "layoutDensity",
      "readingFontScale",
    ]) ||
    !["zh-CN", "en"].includes(String(value.preferences.locale)) ||
    !["SYSTEM", "LIGHT", "DARK"].includes(String(value.preferences.themeMode)) ||
    !["COMPACT", "BALANCED", "SPACIOUS"].includes(
      String(value.preferences.layoutDensity),
    ) ||
    !Number.isInteger(value.preferences.readingFontScale) ||
    Number(value.preferences.readingFontScale) < 85 ||
    Number(value.preferences.readingFontScale) > 130
  ) {
    return null
  }
  return {
    locale: value.preferences.locale as UserPreferences["locale"],
    themeMode: value.preferences.themeMode as UserPreferences["themeMode"],
    layoutDensity:
      value.preferences.layoutDensity as UserPreferences["layoutDensity"],
    readingFontScale: value.preferences.readingFontScale as number,
    readingFontFamily: "SERIF",
    readingCustomFontId: null,
    readingColorScheme: "AUTO",
    linkOpenMode: "NEW_TAB",
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function hasOnlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value)
  return actual.length === keys.length && actual.every((key) => keys.includes(key))
}
