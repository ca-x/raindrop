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
  } catch {
    try {
      localStorage.removeItem(key)
    } catch {
      // The presentation hint is optional when browser storage is unavailable.
    }
  }

  function preferencesFromHint(value) {
    if (!hasOnlyKeys(value, ["schemaVersion", "preferences"])) return null
    if (value.schemaVersion === 2 && isPreferences(value.preferences)) {
      return value.preferences
    }
    if (value.schemaVersion === 1 && isLegacyPreferences(value.preferences)) {
      return {
        ...value.preferences,
        readingFontFamily: "SERIF",
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
      ["AUTO", "PAPER", "SEPIA", "GRAY"].includes(value.readingColorScheme) &&
      ["CURRENT_TAB", "NEW_TAB"].includes(value.linkOpenMode)
    )
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
