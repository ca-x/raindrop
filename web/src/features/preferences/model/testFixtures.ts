import { vi } from "vitest"

import type { PreferencesController } from "./usePreferencesController"

export function fakePreferencesController(
  overrides: Partial<PreferencesController> = {},
): PreferencesController {
  return {
    preferences: {
      locale: "en",
      themeMode: "SYSTEM",
      layoutDensity: "BALANCED",
      readingFontScale: 100,
    },
    loadStatus: "ready",
    error: null,
    isSaving: false,
    load: vi.fn().mockResolvedValue(undefined),
    cancelLoad: vi.fn(),
    save: vi.fn().mockResolvedValue(true),
    clearError: vi.fn(),
    clearHint: vi.fn(),
    ...overrides,
  }
}
