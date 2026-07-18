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
