(() => {
  const key = "raindrop.preferences.v1"

  try {
    const stored = localStorage.getItem(key)
    if (stored === null) return
    const hint = JSON.parse(stored)
    if (!isHint(hint)) {
      localStorage.removeItem(key)
      return
    }

    const preferences = hint.preferences
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
  } catch {
    try {
      localStorage.removeItem(key)
    } catch {
      // The presentation hint is optional when browser storage is unavailable.
    }
  }

  function isHint(value) {
    return (
      hasOnlyKeys(value, ["schemaVersion", "preferences"]) &&
      value.schemaVersion === 1 &&
      isPreferences(value.preferences)
    )
  }

  function isPreferences(value) {
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
