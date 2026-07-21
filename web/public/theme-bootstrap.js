(() => {
  const key = "raindrop.preferences.v1"

  try {
    const stored = localStorage.getItem(key)
    if (stored === null) return
    const hint = JSON.parse(stored)
    const preferences = preferencesFromHint(hint)
    if (preferences === null) {
      localStorage.removeItem(key)
      return
    }

    const root = document.documentElement
    if (preferences.themeMode === "SYSTEM") {
      root.removeAttribute("data-theme")
    } else {
      root.setAttribute("data-theme", preferences.themeMode.toLowerCase())
    }
    root.lang = preferences.locale
    root.dataset.raindropDensity = preferences.layoutDensity.toLowerCase()
    root.style.setProperty(
      "--raindrop-reading-scale",
      `${preferences.readingFontScale}%`,
    )
    root.dataset.raindropReadingFont = preferences.readingFontFamily.toLowerCase()
    root.dataset.raindropReadingColor = preferences.readingColorScheme.toLowerCase()
    if (preferences.readingCustomFontId) {
      const family = `RaindropCustom_${preferences.readingCustomFontId.replaceAll("-", "")}`
      const style = document.createElement("style")
      style.id = "raindrop-custom-reading-font"
      style.textContent = `@font-face{font-family:"${family}";src:url("/api/v2/preferences/fonts/${preferences.readingCustomFontId}/file") format("woff2");font-display:swap;}`
      document.head.append(style)
      root.dataset.raindropReadingCustomFont = preferences.readingCustomFontId
      root.style.setProperty("--raindrop-custom-reading-font", `"${family}"`)
    }
  } catch {
    try {
      localStorage.removeItem(key)
    } catch {
      // The presentation hint is optional when browser storage is unavailable.
    }
  }

  function preferencesFromHint(value) {
    if (!hasOnlyKeys(value, ["schemaVersion", "preferences"])) return null
    if (value.schemaVersion === 3 && isPreferences(value.preferences)) {
      return value.preferences
    }
    if (value.schemaVersion === 2 && isV2Preferences(value.preferences)) {
      return { ...value.preferences, readingCustomFontId: null }
    }
    if (value.schemaVersion === 1 && isLegacyPreferences(value.preferences)) {
      return {
        ...value.preferences,
        readingFontFamily: "SERIF",
        readingCustomFontId: null,
        readingColorScheme: "AUTO",
        linkOpenMode: "NEW_TAB",
      }
    }
    return null
  }

  function isPreferences(value) {
    return (
      hasOnlyKeys(value, [
        "locale",
        "themeMode",
        "layoutDensity",
        "readingFontScale",
        "readingFontFamily",
        "readingCustomFontId",
        "readingColorScheme",
        "linkOpenMode",
      ]) &&
      ["zh-CN", "en"].includes(value.locale) &&
      ["SYSTEM", "LIGHT", "DARK"].includes(value.themeMode) &&
      ["COMPACT", "BALANCED", "SPACIOUS"].includes(value.layoutDensity) &&
      Number.isInteger(value.readingFontScale) &&
      value.readingFontScale >= 85 &&
      value.readingFontScale <= 130 &&
      ["SERIF", "SANS"].includes(value.readingFontFamily) &&
      (value.readingCustomFontId === null || isUuid(value.readingCustomFontId)) &&
      ["AUTO", "PAPER", "SEPIA", "GRAY"].includes(value.readingColorScheme) &&
      ["CURRENT_TAB", "NEW_TAB"].includes(value.linkOpenMode)
    )
  }

  function isV2Preferences(value) {
    return (
      hasOnlyKeys(value, [
        "locale",
        "themeMode",
        "layoutDensity",
        "readingFontScale",
        "readingFontFamily",
        "readingColorScheme",
        "linkOpenMode",
      ]) &&
      isPreferences({ ...value, readingCustomFontId: null })
    )
  }

  function isUuid(value) {
    return typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value)
  }

  function isLegacyPreferences(value) {
    return (
      hasOnlyKeys(value, [
        "locale",
        "themeMode",
        "layoutDensity",
        "readingFontScale",
      ]) &&
      ["zh-CN", "en"].includes(value.locale) &&
      ["SYSTEM", "LIGHT", "DARK"].includes(value.themeMode) &&
      ["COMPACT", "BALANCED", "SPACIOUS"].includes(value.layoutDensity) &&
      Number.isInteger(value.readingFontScale) &&
      value.readingFontScale >= 85 &&
      value.readingFontScale <= 130
    )
  }

  function hasOnlyKeys(value, keys) {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return false
    }
    const actual = Object.keys(value)
    return actual.length === keys.length && actual.every((key) => keys.includes(key))
  }
})()
