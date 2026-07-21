import { vi } from "vitest"

import type { PreferencesController } from "./usePreferencesController"

export function fakePreferencesController(
  overrides: Partial<PreferencesController> = {},
): PreferencesController {
  return {
    csrfToken: "csrf-memory",
    preferences: {
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
      readingFontFamily: "SERIF",
      readingCustomFontId: null,
      readingColorScheme: "AUTO",
      linkOpenMode: "NEW_TAB",
    },
    loadStatus: "ready",
    error: null,
    isSaving: false,
    fonts: [],
    fontLimits: { maximumCount: 8, maximumBytes: 5 * 1024 * 1024 },
    isFontMutating: false,
    load: vi.fn().mockResolvedValue(undefined),
    cancelLoad: vi.fn(),
    save: vi.fn().mockResolvedValue(true),
    uploadFont: vi.fn().mockResolvedValue(true),
    deleteFont: vi.fn().mockResolvedValue(true),
    clearError: vi.fn(),
    clearHint: vi.fn(),
    ...overrides,
  }
}
