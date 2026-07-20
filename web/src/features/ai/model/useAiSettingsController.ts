import { useCallback, useEffect, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  getAiConfig,
  putAiConfig,
} from "../api/content"
import type {
  AiConfigEnvelope,
  PutAiConfigRequest,
} from "../api/content.generated"
import {
  createProvider,
  listProviders,
  updateProvider,
} from "../api/providers"
import type {
  CreateProviderRequest,
  Provider,
  ProviderList,
  ProviderListKeyringStatus,
  UpdateProviderRequest,
} from "../api/provider.generated"
import {
  providerDraftRequest,
  type ProviderDraft,
} from "./providerDraft"

export interface AiSettingsApi {
  listProviders: (signal?: AbortSignal) => Promise<ProviderList>
  createProvider: (
    csrfToken: string,
    request: CreateProviderRequest,
    signal?: AbortSignal,
  ) => Promise<Provider>
  updateProvider: (
    providerId: string,
    csrfToken: string,
    request: UpdateProviderRequest,
    signal?: AbortSignal,
  ) => Promise<Provider>
  getAiConfig: (signal?: AbortSignal) => Promise<AiConfigEnvelope>
  putAiConfig: (
    csrfToken: string,
    request: PutAiConfigRequest,
    signal?: AbortSignal,
  ) => Promise<AiConfigEnvelope>
}

export type AiSettingsLoadStatus = "idle" | "loading" | "ready" | "error"
export type AiSettingsError =
  | "LOAD"
  | "PROVIDER_VALIDATION"
  | "PROVIDER_SAVE"
  | "PROVIDER_CONFLICT"
  | "PROVIDER_KEYRING"
  | "CONFIG_SAVE"
  | "CONFIG_CONFLICT"
  | "CONFIG_UNAVAILABLE"

export interface AiSettingsController {
  csrfToken: string
  providers: Provider[]
  keyringStatus: ProviderListKeyringStatus | null
  configEnvelope: AiConfigEnvelope | null
  loadStatus: AiSettingsLoadStatus
  error: AiSettingsError | null
  isSavingProvider: boolean
  isSavingConfig: boolean
  load: () => Promise<void>
  saveProvider: (draft: ProviderDraft) => Promise<boolean>
  saveConfig: (request: PutAiConfigRequest) => Promise<boolean>
  cancel: () => void
  clearError: () => void
}

interface UseAiSettingsControllerOptions {
  csrfToken: string
  onUnauthenticated: () => void
  api?: AiSettingsApi
}

const defaultApi: AiSettingsApi = {
  listProviders,
  createProvider,
  updateProvider,
  getAiConfig,
  putAiConfig,
}

export function useAiSettingsController({
  csrfToken,
  onUnauthenticated,
  api = defaultApi,
}: UseAiSettingsControllerOptions): AiSettingsController {
  const [providers, setProviders] = useState<Provider[]>([])
  const [keyringStatus, setKeyringStatus] =
    useState<ProviderListKeyringStatus | null>(null)
  const [configEnvelope, setConfigEnvelope] = useState<AiConfigEnvelope | null>(null)
  const [loadStatus, setLoadStatus] = useState<AiSettingsLoadStatus>("idle")
  const [error, setError] = useState<AiSettingsError | null>(null)
  const [isSavingProvider, setIsSavingProvider] = useState(false)
  const [isSavingConfig, setIsSavingConfig] = useState(false)
  const loadGeneration = useRef(0)
  const loadAbort = useRef<AbortController | null>(null)
  const providerAbort = useRef<AbortController | null>(null)
  const configAbort = useRef<AbortController | null>(null)
  const savingProvider = useRef(false)
  const savingConfig = useRef(false)

  const endSession = useCallback(() => {
    loadGeneration.current += 1
    loadAbort.current?.abort()
    providerAbort.current?.abort()
    configAbort.current?.abort()
    onUnauthenticated()
  }, [onUnauthenticated])

  const cancel = useCallback(() => {
    loadGeneration.current += 1
    loadAbort.current?.abort()
    providerAbort.current?.abort()
    configAbort.current?.abort()
    loadAbort.current = null
    providerAbort.current = null
    configAbort.current = null
  }, [])

  useEffect(() => cancel, [cancel])

  const load = useCallback(async () => {
    loadAbort.current?.abort()
    const generation = ++loadGeneration.current
    const abort = new AbortController()
    loadAbort.current = abort
    setLoadStatus("loading")
    setError(null)
    try {
      const [providerState, configState] = await Promise.all([
        api.listProviders(abort.signal),
        api.getAiConfig(abort.signal),
      ])
      if (generation !== loadGeneration.current) return
      setProviders(providerState.items)
      setKeyringStatus(providerState.keyringStatus)
      setConfigEnvelope(configState)
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

  const saveProvider = useCallback(
    async (draft: ProviderDraft) => {
      if (savingProvider.current) return false
      const parsed = providerDraftRequest(draft)
      if (!parsed.ok) {
        setError("PROVIDER_VALIDATION")
        return false
      }

      const abort = new AbortController()
      providerAbort.current = abort
      savingProvider.current = true
      setIsSavingProvider(true)
      setError(null)
      try {
        const persisted =
          parsed.mode === "create"
            ? await api.createProvider(csrfToken, parsed.request, abort.signal)
            : await api.updateProvider(
                parsed.providerId,
                csrfToken,
                parsed.request,
                abort.signal,
              )
        setProviders((current) => upsertProvider(current, persisted))
        if (
          keyringStatus === "UNAVAILABLE" &&
          (parsed.mode === "create" || parsed.request.credential !== undefined)
        ) {
          setKeyringStatus("AVAILABLE")
        }
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        const mapped = providerError(cause)
        if (mapped === "PROVIDER_KEYRING") setKeyringStatus("UNAVAILABLE")
        setError(mapped)
        return false
      } finally {
        if (providerAbort.current === abort) providerAbort.current = null
        savingProvider.current = false
        setIsSavingProvider(false)
      }
    },
    [api, csrfToken, endSession, keyringStatus],
  )

  const saveConfig = useCallback(
    async (request: PutAiConfigRequest) => {
      if (savingConfig.current) return false
      const abort = new AbortController()
      configAbort.current = abort
      savingConfig.current = true
      setIsSavingConfig(true)
      setError(null)
      try {
        const persisted = await api.putAiConfig(csrfToken, request, abort.signal)
        setConfigEnvelope(persisted)
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        setError(configError(cause))
        return false
      } finally {
        if (configAbort.current === abort) configAbort.current = null
        savingConfig.current = false
        setIsSavingConfig(false)
      }
    },
    [api, csrfToken, endSession],
  )

  return {
    csrfToken,
    providers,
    keyringStatus,
    configEnvelope,
    loadStatus,
    error,
    isSavingProvider,
    isSavingConfig,
    load,
    saveProvider,
    saveConfig,
    cancel,
    clearError: useCallback(() => setError(null), []),
  }
}

function upsertProvider(current: Provider[], persisted: Provider): Provider[] {
  const index = current.findIndex(
    (provider) => provider.providerId === persisted.providerId,
  )
  if (index < 0) return [...current, persisted]
  return current.map((provider, itemIndex) =>
    itemIndex === index ? persisted : provider,
  )
}

function providerError(error: unknown): AiSettingsError {
  if (!(error instanceof ApiClientError)) return "PROVIDER_SAVE"
  if (error.status === 409 && error.payload.code === "REVISION_CONFLICT") {
    return "PROVIDER_CONFLICT"
  }
  if (error.status === 503 && error.payload.code === "AI_PROVIDER_KEYRING_UNAVAILABLE") {
    return "PROVIDER_KEYRING"
  }
  return "PROVIDER_SAVE"
}

function configError(error: unknown): AiSettingsError {
  if (!(error instanceof ApiClientError)) return "CONFIG_SAVE"
  if (error.status === 409 && error.payload.code === "REVISION_CONFLICT") {
    return "CONFIG_CONFLICT"
  }
  if (
    error.payload.code === "AI_PLUGIN_UNAVAILABLE" ||
    error.status === 404
  ) {
    return "CONFIG_UNAVAILABLE"
  }
  return "CONFIG_SAVE"
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
