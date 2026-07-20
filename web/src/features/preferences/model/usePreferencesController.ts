import { useCallback, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  getPreferences,
  patchPreferences,
} from "../api/preferences"
import type {
  PatchUserPreferencesRequest,
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
}

export type PreferencesControllerError = "LOAD" | "SAVE"
export type PreferencesLoadStatus = "idle" | "loading" | "ready" | "error"

export interface PreferencesController {
  csrfToken: string
  preferences: UserPreferences
  loadStatus: PreferencesLoadStatus
  error: PreferencesControllerError | null
  isSaving: boolean
  load: () => Promise<void>
  cancelLoad: () => void
  save: (draft: UserPreferences) => Promise<boolean>
  clearError: () => void
  clearHint: () => void
}

interface UsePreferencesControllerOptions {
  csrfToken: string
  onUnauthenticated: () => void
  api?: PreferencesApi
}

const defaultApi: PreferencesApi = { getPreferences, patchPreferences }

export function usePreferencesController({
  csrfToken,
  onUnauthenticated,
  api = defaultApi,
}: UsePreferencesControllerOptions): PreferencesController {
  const runtime = usePreferenceRuntime()
  const [loadStatus, setLoadStatus] = useState<PreferencesLoadStatus>("idle")
  const [error, setError] = useState<PreferencesControllerError | null>(null)
  const [isSaving, setIsSaving] = useState(false)
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
      const preferences = await api.getPreferences(abort.signal)
      if (generation !== loadGeneration.current) return
      runtime.apply(preferences)
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

  return {
    csrfToken,
    preferences: runtime.preferences,
    loadStatus,
    error,
    isSaving,
    load,
    cancelLoad,
    save,
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
