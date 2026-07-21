import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"

import {
  activateLocale,
  detectLocale,
  i18n,
  type AppLocale,
} from "../../../shared/i18n/i18n"
import {
  isUserPreferences,
  type UserPreferences,
} from "../api/preferences.generated"
import {
  clearPreferenceHint,
  readPreferenceHint,
  writePreferenceHint,
} from "./preferenceCache"
import {
  defaultPreferences,
  toDensityAttribute,
  toReadingDataValue,
  toReadingScaleCss,
} from "./preferenceTypes"

interface PreferenceRuntimeValue {
  preferences: UserPreferences
  apply: (preferences: UserPreferences) => void
  clearHint: () => void
}

const PreferenceRuntimeContext = createContext<PreferenceRuntimeValue | null>(null)

export function PreferenceRuntimeProvider({ children }: { children: ReactNode }) {
  const [preferences, setPreferences] = useState(initialPreferences)

  useLayoutEffect(() => {
    activateLocale(preferences.locale)
    document.documentElement.dataset.raindropDensity = toDensityAttribute(
      preferences.layoutDensity,
    )
    document.documentElement.style.setProperty(
      "--raindrop-reading-scale",
      toReadingScaleCss(preferences.readingFontScale),
    )
    document.documentElement.dataset.raindropReadingFont = toReadingDataValue(
      preferences.readingFontFamily,
    )
    applyCustomReadingFont(preferences.readingCustomFontId)
    document.documentElement.dataset.raindropReadingColor = toReadingDataValue(
      preferences.readingColorScheme,
    )
  }, [preferences])

  const apply = useCallback((next: UserPreferences) => {
    if (!isUserPreferences(next)) return
    writePreferenceHint(next)
    setPreferences(next)
  }, [])
  const clearHint = useCallback(() => clearPreferenceHint(), [])
  const value = useMemo(
    () => ({ preferences, apply, clearHint }),
    [apply, clearHint, preferences],
  )

  return (
    <PreferenceRuntimeContext.Provider value={value}>
      {children}
    </PreferenceRuntimeContext.Provider>
  )
}

function applyCustomReadingFont(fontId: string | null): void {
  const root = document.documentElement
  const styleId = "raindrop-custom-reading-font"
  document.getElementById(styleId)?.remove()
  if (!fontId) {
    delete root.dataset.raindropReadingCustomFont
    root.style.removeProperty("--raindrop-custom-reading-font")
    return
  }
  const family = `RaindropCustom_${fontId.replaceAll("-", "")}`
  const style = document.createElement("style")
  style.id = styleId
  style.textContent = `@font-face{font-family:"${family}";src:url("/api/v2/preferences/fonts/${fontId}/file") format("woff2");font-display:swap;}`
  document.head.append(style)
  root.dataset.raindropReadingCustomFont = fontId
  root.style.setProperty("--raindrop-custom-reading-font", `"${family}"`)
}

export function usePreferenceRuntime(): PreferenceRuntimeValue {
  const runtime = useContext(PreferenceRuntimeContext)
  if (!runtime) {
    throw new Error(
      "usePreferenceRuntime must be used within PreferenceRuntimeProvider",
    )
  }
  return runtime
}

function initialPreferences(): UserPreferences {
  return readPreferenceHint() ?? defaultPreferences(currentLocale())
}

function currentLocale(): AppLocale {
  return i18n.locale === "zh-CN" || i18n.locale === "en"
    ? i18n.locale
    : detectLocale()
}
