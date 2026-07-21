import { useCallback, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  deleteUserFont,
  getPreferences,
  listUserFonts,
  patchPreferences,
  uploadUserFont,
} from "../api/preferences"
import type {
  PatchUserPreferencesRequest,
  UserFont,
  UserFontList,
  UserPreferences,
} from "../api/preferences.generated"
import { usePreferenceRuntime } from "./PreferenceRuntime"

export interface PreferencesApi {
  getPreferences: (signal?: AbortSignal) => Promise<UserPreferences>
  patchPreferences: (
    csrfToken: string,
    patch: PatchUserPreferencesRequest,
    signal?: AbortSignal,
  ) => Promise<UserPreferences>
  listUserFonts: (signal?: AbortSignal) => Promise<UserFontList>
  uploadUserFont: (file: File, csrfToken: string, signal?: AbortSignal) => Promise<UserFont>
  deleteUserFont: (fontId: string, csrfToken: string, signal?: AbortSignal) => Promise<void>
}

export type PreferencesControllerError = "LOAD" | "SAVE"
export type PreferencesLoadStatus = "idle" | "loading" | "ready" | "error"

export interface PreferencesController {
  csrfToken: string
  preferences: UserPreferences
  loadStatus: PreferencesLoadStatus
  error: PreferencesControllerError | null
  isSaving: boolean
  fonts: UserFont[]
  fontLimits: { maximumCount: number; maximumBytes: number }
  isFontMutating: boolean
  load: () => Promise<void>
  cancelLoad: () => void
  save: (draft: UserPreferences) => Promise<boolean>
  uploadFont: (file: File) => Promise<boolean>
  deleteFont: (fontId: string) => Promise<boolean>
  clearError: () => void
  clearHint: () => void
}

interface UsePreferencesControllerOptions {
  csrfToken: string
  onUnauthenticated: () => void
  api?: PreferencesApi
}

const defaultApi: PreferencesApi = {
  getPreferences,
  patchPreferences,
  listUserFonts,
  uploadUserFont,
  deleteUserFont,
}

export function usePreferencesController({
  csrfToken,
  onUnauthenticated,
  api = defaultApi,
}: UsePreferencesControllerOptions): PreferencesController {
  const runtime = usePreferenceRuntime()
  const [loadStatus, setLoadStatus] = useState<PreferencesLoadStatus>("idle")
  const [error, setError] = useState<PreferencesControllerError | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [fonts, setFonts] = useState<UserFont[]>([])
  const [fontLimits, setFontLimits] = useState({ maximumCount: 8, maximumBytes: 5 * 1024 * 1024 })
  const [isFontMutating, setIsFontMutating] = useState(false)
  const isSavingRef = useRef(false)
  const loadGeneration = useRef(0)
  const loadAbort = useRef<AbortController | null>(null)

  const endSession = useCallback(() => {
    runtime.clearHint()
    onUnauthenticated()
  }, [onUnauthenticated, runtime.clearHint])

  const load = useCallback(async () => {
    loadAbort.current?.abort()
    const generation = ++loadGeneration.current
    const abort = new AbortController()
    loadAbort.current = abort
    setLoadStatus("loading")
    setError(null)
    try {
      const [preferences, fontList] = await Promise.all([
        api.getPreferences(abort.signal),
        api.listUserFonts(abort.signal),
      ])
      if (generation !== loadGeneration.current) return
      runtime.apply(preferences)
      setFonts(fontList.items)
      setFontLimits({
        maximumCount: fontList.maximumCount,
        maximumBytes: fontList.maximumBytes,
      })
      setLoadStatus("ready")
    } catch (cause) {
      if (generation !== loadGeneration.current || isAbortError(cause)) return
      if (isAuthenticationError(cause)) {
        endSession()
        return
      }
      setLoadStatus("error")
      setError("LOAD")
    } finally {
      if (generation === loadGeneration.current) loadAbort.current = null
    }
  }, [api, endSession, runtime.apply])

  const cancelLoad = useCallback(() => {
    loadGeneration.current += 1
    loadAbort.current?.abort()
    loadAbort.current = null
  }, [])

  const save = useCallback(
    async (draft: UserPreferences) => {
      if (isSavingRef.current) return false
      const patch = preferencePatch(runtime.preferences, draft)
      if (!patch) {
        setError(null)
        return true
      }

      const previous = runtime.preferences
      const abort = new AbortController()
      isSavingRef.current = true
      setIsSaving(true)
      setError(null)
      runtime.apply(draft)
      try {
        const persisted = await api.patchPreferences(csrfToken, patch, abort.signal)
        runtime.apply(persisted)
        return true
      } catch (cause) {
        runtime.apply(previous)
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        setError("SAVE")
        return false
      } finally {
        isSavingRef.current = false
        setIsSaving(false)
      }
    },
    [api, csrfToken, endSession, runtime.apply, runtime.preferences],
  )

  const uploadFont = useCallback(async (file: File) => {
    if (isFontMutating) return false
    setIsFontMutating(true)
    setError(null)
    try {
      const font = await api.uploadUserFont(file, csrfToken)
      setFonts((current) => [...current, font])
      return true
    } catch (cause) {
      if (isAuthenticationError(cause)) endSession()
      else setError("SAVE")
      return false
    } finally {
      setIsFontMutating(false)
    }
  }, [api, csrfToken, endSession, isFontMutating])

  const deleteFont = useCallback(async (fontId: string) => {
    if (isFontMutating) return false
    setIsFontMutating(true)
    setError(null)
    try {
      await api.deleteUserFont(fontId, csrfToken)
      setFonts((current) => current.filter((font) => font.fontId !== fontId))
      if (runtime.preferences.readingCustomFontId === fontId) {
        runtime.apply({ ...runtime.preferences, readingCustomFontId: null })
      }
      return true
    } catch (cause) {
      if (isAuthenticationError(cause)) endSession()
      else setError("SAVE")
      return false
    } finally {
      setIsFontMutating(false)
    }
  }, [api, csrfToken, endSession, isFontMutating, runtime])

  return {
    csrfToken,
    preferences: runtime.preferences,
    loadStatus,
    error,
    isSaving,
    fonts,
    fontLimits,
    isFontMutating,
    load,
    cancelLoad,
    save,
    uploadFont,
    deleteFont,
    clearError: useCallback(() => setError(null), []),
    clearHint: runtime.clearHint,
  }
}

function preferencePatch(
  current: UserPreferences,
  draft: UserPreferences,
): PatchUserPreferencesRequest | null {
  const patch: Partial<UserPreferences> = {}
  if (current.locale !== draft.locale) patch.locale = draft.locale
  if (current.themeMode !== draft.themeMode) patch.themeMode = draft.themeMode
  if (current.layoutDensity !== draft.layoutDensity) {
    patch.layoutDensity = draft.layoutDensity
  }
  if (current.readingFontScale !== draft.readingFontScale) {
    patch.readingFontScale = draft.readingFontScale
  }
  if (current.readingFontFamily !== draft.readingFontFamily) {
    patch.readingFontFamily = draft.readingFontFamily
  }
  if (current.readingCustomFontId !== draft.readingCustomFontId) {
    patch.readingCustomFontId = draft.readingCustomFontId
  }
  if (current.readingColorScheme !== draft.readingColorScheme) {
    patch.readingColorScheme = draft.readingColorScheme
  }
  if (current.linkOpenMode !== draft.linkOpenMode) {
    patch.linkOpenMode = draft.linkOpenMode
  }
  return Object.keys(patch).length > 0
    ? (patch as PatchUserPreferencesRequest)
    : null
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
