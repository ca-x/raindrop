import {
  isUserPreferences,
  type UserPreferences,
} from "../api/preferences.generated"

export const PREFERENCE_HINT_KEY = "raindrop.preferences.v1"

interface PreferenceHintV1 {
  schemaVersion: 1
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
    if (!isPreferenceHint(parsed)) {
      clearPreferenceHint()
      return null
    }
    return parsed.preferences
  } catch {
    clearPreferenceHint()
    return null
  }
}

export function writePreferenceHint(preferences: UserPreferences): void {
  if (!isUserPreferences(preferences)) return
  const hint: PreferenceHintV1 = { schemaVersion: 1, preferences }
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

function isPreferenceHint(value: unknown): value is PreferenceHintV1 {
  return (
    isRecord(value) &&
    hasOnlyKeys(value, ["schemaVersion", "preferences"]) &&
    value.schemaVersion === 1 &&
    isUserPreferences(value.preferences)
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function hasOnlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value)
  return actual.length === keys.length && actual.every((key) => keys.includes(key))
}
