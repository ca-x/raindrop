import type { ThemeMode as AstryxThemeMode } from "@astryxdesign/core/theme"
import type { ListDensity } from "@astryxdesign/core/List"

import type {
  UserPreferences,
  UserPreferencesLayoutDensity,
  UserPreferencesLocale,
  UserPreferencesThemeMode,
} from "../api/preferences.generated"

export type PreferenceLocale = UserPreferencesLocale
export type PreferenceThemeMode = UserPreferencesThemeMode
export type PreferenceLayoutDensity = UserPreferencesLayoutDensity

export function defaultPreferences(locale: PreferenceLocale): UserPreferences {
  return {
    locale,
    themeMode: "SYSTEM",
    layoutDensity: "BALANCED",
    readingFontScale: 100,
    readingFontFamily: "SERIF",
    readingCustomFontId: null,
    readingColorScheme: "AUTO",
    linkOpenMode: "NEW_TAB",
  }
}

export function toAstryxThemeMode(mode: PreferenceThemeMode): AstryxThemeMode {
  switch (mode) {
    case "SYSTEM":
      return "system"
    case "LIGHT":
      return "light"
    case "DARK":
      return "dark"
  }
}

export function toDensityAttribute(density: PreferenceLayoutDensity): string {
  return density.toLowerCase()
}

export function toAstryxDensity(density: PreferenceLayoutDensity): ListDensity {
  return toDensityAttribute(density) as ListDensity
}

export function toReadingScaleCss(scale: number): string {
  return `${scale}%`
}

export function toReadingDataValue(value: string): string {
  return value.toLowerCase().replace("_", "-")
}
