import { useCallback, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  getTranslationConfig,
  putTranslationConfig,
  testTranslationConnection,
} from "../api/translation"
import type {
  PutTranslationConfigRequest,
  TestTranslationRequest,
  TranslationConfig,
  TranslationDisplayMode,
  TranslationTestResult,
} from "../api/translation.generated"

export interface TranslationSettingsApi {
  getConfig: (signal?: AbortSignal) => Promise<TranslationConfig>
  putConfig: (
    csrfToken: string,
    request: PutTranslationConfigRequest,
    signal?: AbortSignal,
  ) => Promise<TranslationConfig>
  testConnection: (
    csrfToken: string,
    request: TestTranslationRequest,
    signal?: AbortSignal,
  ) => Promise<TranslationTestResult>
}

export type TranslationSettingsError =
  | "LOAD"
  | "VALIDATION"
  | "SAVE"
  | "CONFLICT"
  | "PROVIDER_UNAVAILABLE"
  | "SECRET_UNAVAILABLE"
  | "TEST"
  | "RATE_LIMITED"

export interface TranslationSettingsController {
  config: TranslationConfig | null
  loadStatus: "idle" | "loading" | "ready" | "error"
  error: TranslationSettingsError | null
  isSaving: boolean
  isTesting: boolean
  testResult: TranslationTestResult | null
  load: () => Promise<void>
  save: (request: PutTranslationConfigRequest) => Promise<boolean>
  saveDisplayMode: (mode: TranslationDisplayMode) => Promise<boolean>
  test: (request: TestTranslationRequest) => Promise<boolean>
  clearError: () => void
  cancel: () => void
}

interface Options {
  csrfToken: string
  onUnauthenticated: () => void
  api?: TranslationSettingsApi
}

const defaultApi: TranslationSettingsApi = {
  getConfig: getTranslationConfig,
  putConfig: putTranslationConfig,
  testConnection: testTranslationConnection,
}

export function useTranslationSettingsController({
  csrfToken,
  onUnauthenticated,
  api = defaultApi,
}: Options): TranslationSettingsController {
  const [config, setConfig] = useState<TranslationConfig | null>(null)
  const [loadStatus, setLoadStatus] =
    useState<TranslationSettingsController["loadStatus"]>("idle")
  const [error, setError] = useState<TranslationSettingsError | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [isTesting, setIsTesting] = useState(false)
  const [testResult, setTestResult] = useState<TranslationTestResult | null>(null)
  const loadGeneration = useRef(0)
  const loadAbort = useRef<AbortController | null>(null)
  const mutationAbort = useRef<AbortController | null>(null)
  const saving = useRef(false)
  const testing = useRef(false)

  const endSession = useCallback(() => {
    loadGeneration.current += 1
    loadAbort.current?.abort()
    mutationAbort.current?.abort()
    onUnauthenticated()
  }, [onUnauthenticated])

  const cancel = useCallback(() => {
    loadGeneration.current += 1
    loadAbort.current?.abort()
    mutationAbort.current?.abort()
    loadAbort.current = null
    mutationAbort.current = null
  }, [])

  const load = useCallback(async () => {
    loadAbort.current?.abort()
    const generation = ++loadGeneration.current
    const abort = new AbortController()
    loadAbort.current = abort
    setLoadStatus("loading")
    setError(null)
    try {
      const loaded = await api.getConfig(abort.signal)
      if (generation !== loadGeneration.current) return
      setConfig(loaded)
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
  }, [api, endSession])

  const save = useCallback(
    async (request: PutTranslationConfigRequest) => {
      if (saving.current) return false
      const abort = new AbortController()
      mutationAbort.current?.abort()
      mutationAbort.current = abort
      saving.current = true
      setIsSaving(true)
      setError(null)
      try {
        const persisted = await api.putConfig(csrfToken, request, abort.signal)
        setConfig(persisted)
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        setError(mapError(cause, "SAVE"))
        return false
      } finally {
        if (mutationAbort.current === abort) mutationAbort.current = null
        saving.current = false
        setIsSaving(false)
      }
    },
    [api, csrfToken, endSession],
  )

  const saveDisplayMode = useCallback(
    async (displayMode: TranslationDisplayMode) => {
      if (!config || config.displayMode === displayMode) return true
      const request = configRequest(config)
      request.displayMode = displayMode
      return save(request)
    },
    [config, save],
  )

  const test = useCallback(
    async (request: TestTranslationRequest) => {
      if (testing.current) return false
      const abort = new AbortController()
      mutationAbort.current?.abort()
      mutationAbort.current = abort
      testing.current = true
      setIsTesting(true)
      setError(null)
      setTestResult(null)
      try {
        const result = await api.testConnection(csrfToken, request, abort.signal)
        setTestResult(result)
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        setError(mapError(cause, "TEST"))
        return false
      } finally {
        if (mutationAbort.current === abort) mutationAbort.current = null
        testing.current = false
        setIsTesting(false)
      }
    },
    [api, csrfToken, endSession],
  )

  return {
    config,
    loadStatus,
    error,
    isSaving,
    isTesting,
    testResult,
    load,
    save,
    saveDisplayMode,
    test,
    clearError: useCallback(() => {
      setError(null)
      setTestResult(null)
    }, []),
    cancel,
  }
}

export function configRequest(config: TranslationConfig): PutTranslationConfigRequest {
  return {
    expectedRevision: config.revision,
    engine: config.engine,
    displayMode: config.displayMode,
    isEnabled: config.isEnabled,
    defaultTargetLocale: config.defaultTargetLocale,
    openAi: { ...config.openAi },
    deepLx: {
      displayName: config.deepLx.displayName,
      description: config.deepLx.description,
      baseUrl: config.deepLx.baseUrl,
      isProgressive: config.deepLx.isProgressive,
    },
  }
}

function mapError(
  cause: unknown,
  fallback: "SAVE" | "TEST",
): TranslationSettingsError {
  if (!(cause instanceof ApiClientError)) return fallback
  if (cause.status === 422) return "VALIDATION"
  if (cause.payload.code === "REVISION_CONFLICT") return "CONFLICT"
  if (cause.payload.code === "TRANSLATION_PROVIDER_UNAVAILABLE") {
    return "PROVIDER_UNAVAILABLE"
  }
  if (cause.payload.code === "TRANSLATION_SECRET_UNAVAILABLE") {
    return "SECRET_UNAVAILABLE"
  }
  if (cause.status === 429) return "RATE_LIMITED"
  return fallback
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
