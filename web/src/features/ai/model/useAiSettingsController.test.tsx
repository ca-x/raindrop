import { act, renderHook } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import { ApiClientError } from "../../../shared/api/client"
import type { AiConfigEnvelope, PutAiConfigRequest } from "../api/content.generated"
import type {
  CreateProviderRequest,
  Provider,
  ProviderList,
  UpdateProviderRequest,
} from "../api/provider.generated"
import { createProviderDraft, editProviderDraft } from "./providerDraft"
import {
  useAiSettingsController,
  type AiSettingsApi,
} from "./useAiSettingsController"

const providerId = "00000000-0000-4000-8000-000000000101"
const provider: Provider = {
  providerId,
  scope: "USER",
  canEdit: true,
  displayName: "Primary model",
  kind: "OPENAI_RESPONSES",
  endpoint: "https://api.openai.com/",
  model: "gpt-5-mini",
  capabilities: { supportsUsage: true, supportsIdempotency: true },
  policy: {
    maxConcurrency: 2,
    requestsPerMinute: 60,
    maxInputTokensPerRequest: 32_768,
    maxOutputTokensPerRequest: 4096,
    inputCostMicrosPerMillionTokens: null,
    outputCostMicrosPerMillionTokens: null,
    maxCostMicrosPerRequest: 250_000,
  },
  isEnabled: true,
  revision: 0,
  createdAt: "2026-07-20T10:00:00Z",
  updatedAt: "2026-07-20T10:00:00Z",
}
const providerList: ProviderList = {
  keyringStatus: "AVAILABLE",
  items: [provider],
}
const configEnvelope: AiConfigEnvelope = {
  pluginState: "READY",
  mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
  config: null,
}

it("loads Provider and config state in parallel with shared cancellation", async () => {
  const providers = deferred<ProviderList>()
  const config = deferred<AiConfigEnvelope>()
  const api = fakeApi({
    listProviders: vi.fn(() => providers.promise),
    getAiConfig: vi.fn(() => config.promise),
  })
  const { result } = renderController(api)

  let load!: Promise<void>
  act(() => {
    load = result.current.load()
  })
  expect(api.listProviders).toHaveBeenCalledOnce()
  expect(api.getAiConfig).toHaveBeenCalledOnce()
  expect(result.current.loadStatus).toBe("loading")
  expect(vi.mocked(api.listProviders).mock.calls[0]?.[0]).toBeInstanceOf(AbortSignal)
  expect(vi.mocked(api.getAiConfig).mock.calls[0]?.[0]).toBe(
    vi.mocked(api.listProviders).mock.calls[0]?.[0],
  )

  providers.resolve(providerList)
  config.resolve(configEnvelope)
  await act(async () => load)

  expect(result.current.loadStatus).toBe("ready")
  expect(result.current.providers).toEqual([provider])
  expect(result.current.keyringStatus).toBe("AVAILABLE")
  expect(result.current.configEnvelope).toEqual(configEnvelope)
})

it("aborts in-flight work on cancel and ignores late responses", async () => {
  const providers = deferred<ProviderList>()
  const config = deferred<AiConfigEnvelope>()
  const api = fakeApi({
    listProviders: vi.fn(() => providers.promise),
    getAiConfig: vi.fn(() => config.promise),
  })
  const { result, unmount } = renderController(api)

  let load!: Promise<void>
  act(() => {
    load = result.current.load()
  })
  const signal = vi.mocked(api.listProviders).mock.calls[0]?.[0]
  act(() => result.current.cancel())
  expect(signal?.aborted).toBe(true)

  providers.resolve(providerList)
  config.resolve(configEnvelope)
  await act(async () => load)
  expect(result.current.providers).toEqual([])

  act(() => {
    void result.current.load()
  })
  const nextSignal = vi.mocked(api.listProviders).mock.calls[1]?.[0]
  unmount()
  expect(nextSignal?.aborted).toBe(true)
})

it("creates and replaces Providers without retaining credentials", async () => {
  const created = { ...provider, revision: 0 }
  const replaced = { ...provider, displayName: "Renamed", revision: 1 }
  const api = fakeApi({
    createProvider: vi.fn().mockResolvedValue(created),
    updateProvider: vi.fn().mockResolvedValue(replaced),
  })
  const { result } = renderController(api)
  await act(async () => result.current.load())

  const createDraft = {
    ...createProviderDraft(),
    displayName: "Primary model",
    model: "gpt-5-mini",
    credential: "credential-never-cached",
  }
  await act(async () => {
    await expect(result.current.saveProvider(createDraft)).resolves.toBe(true)
  })
  expect(api.createProvider).toHaveBeenCalledWith(
    "csrf-memory",
    expect.objectContaining({ credential: "credential-never-cached" }),
    expect.any(AbortSignal),
  )
  expect(JSON.stringify(result.current.providers)).not.toContain(
    "credential-never-cached",
  )

  const editDraft = { ...editProviderDraft(created), displayName: "Renamed" }
  await act(async () => {
    await expect(result.current.saveProvider(editDraft)).resolves.toBe(true)
  })
  expect(api.updateProvider).toHaveBeenCalledWith(
    providerId,
    "csrf-memory",
    expect.objectContaining({ expectedRevision: 0, displayName: "Renamed" }),
    expect.any(AbortSignal),
  )
  expect(result.current.providers).toEqual([replaced])
})

it("preserves Provider state and exposes revision conflict", async () => {
  const api = fakeApi({
    updateProvider: vi.fn().mockRejectedValue(
      new ApiClientError(409, {
        code: "REVISION_CONFLICT",
        message: "Conflict",
      }),
    ),
  })
  const { result } = renderController(api)
  await act(async () => result.current.load())

  await act(async () => {
    await expect(
      result.current.saveProvider({
        ...editProviderDraft(provider),
        displayName: "Keep this draft",
      }),
    ).resolves.toBe(false)
  })

  expect(result.current.providers).toEqual([provider])
  expect(result.current.error).toBe("PROVIDER_CONFLICT")
})

it("keeps an unavailable keyring state after a metadata-only Provider update", async () => {
  const replaced = { ...provider, displayName: "Renamed", revision: 1 }
  const api = fakeApi({
    listProviders: vi.fn().mockResolvedValue({
      keyringStatus: "UNAVAILABLE",
      items: [provider],
    }),
    updateProvider: vi.fn().mockResolvedValue(replaced),
  })
  const { result } = renderController(api)
  await act(async () => result.current.load())

  await act(async () => {
    await expect(
      result.current.saveProvider({
        ...editProviderDraft(provider),
        displayName: "Renamed",
      }),
    ).resolves.toBe(true)
  })

  expect(result.current.providers).toEqual([replaced])
  expect(result.current.keyringStatus).toBe("UNAVAILABLE")
})

it("creates and replaces config while surfacing plugin and keyring availability", async () => {
  const unavailableEnvelope: AiConfigEnvelope = {
    pluginState: "QUARANTINED",
    mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
    config: null,
  }
  const api = fakeApi({
    listProviders: vi.fn().mockResolvedValue({
      keyringStatus: "UNAVAILABLE",
      items: [provider],
    }),
    getAiConfig: vi.fn().mockResolvedValue(unavailableEnvelope),
  })
  const { result } = renderController(api)
  await act(async () => result.current.load())

  expect(result.current.keyringStatus).toBe("UNAVAILABLE")
  expect(result.current.configEnvelope?.pluginState).toBe("QUARANTINED")

  const created: AiConfigEnvelope = {
    ...configEnvelope,
    config: configRequest(0, null),
  }
  const replaced: AiConfigEnvelope = {
    ...configEnvelope,
    config: configRequest(1, 0),
  }
  vi.mocked(api.putAiConfig)
    .mockResolvedValueOnce(created)
    .mockResolvedValueOnce(replaced)

  await act(async () => {
    await expect(result.current.saveConfig(configRequestBody(null))).resolves.toBe(true)
  })
  expect(result.current.configEnvelope).toEqual(created)
  await act(async () => {
    await expect(result.current.saveConfig(configRequestBody(0))).resolves.toBe(true)
  })
  expect(result.current.configEnvelope).toEqual(replaced)
})

it("propagates authentication failure without exposing transport details", async () => {
  const onUnauthenticated = vi.fn()
  const api = fakeApi({
    listProviders: vi.fn().mockRejectedValue(
      new ApiClientError(401, {
        code: "AUTHENTICATION_REQUIRED",
        message: "Authentication is required",
      }),
    ),
  })
  const { result } = renderController(api, onUnauthenticated)

  await act(async () => result.current.load())

  expect(onUnauthenticated).toHaveBeenCalledOnce()
  expect(result.current.error).toBeNull()
})

function renderController(api: AiSettingsApi, onUnauthenticated = vi.fn()) {
  return renderHook(() =>
    useAiSettingsController({
      csrfToken: "csrf-memory",
      onUnauthenticated,
      api,
    }),
  )
}

function fakeApi(overrides: Partial<AiSettingsApi> = {}): AiSettingsApi {
  return {
    listProviders: vi.fn().mockResolvedValue(providerList),
    createProvider: vi.fn<
      (
        csrfToken: string,
        request: CreateProviderRequest,
        signal?: AbortSignal,
      ) => Promise<Provider>
    >(),
    updateProvider: vi.fn<
      (
        providerId: string,
        csrfToken: string,
        request: UpdateProviderRequest,
        signal?: AbortSignal,
      ) => Promise<Provider>
    >(),
    getAiConfig: vi.fn().mockResolvedValue(configEnvelope),
    putAiConfig: vi.fn<
      (
        csrfToken: string,
        request: PutAiConfigRequest,
        signal?: AbortSignal,
      ) => Promise<AiConfigEnvelope>
    >(),
    ...overrides,
  }
}

function configRequestBody(expectedRevision: number | null): PutAiConfigRequest {
  return {
    expectedRevision,
    isEnabled: true,
    summary: {
      enabled: true,
      providerId,
      style: "BALANCED",
      maxOutputTokens: 1024,
    },
    translation: {
      enabled: false,
      providerId,
      defaultTargetLocale: "zh-CN",
      maxOutputTokens: 4096,
    },
  }
}

function configRequest(revision: number, expectedRevision: number | null) {
  const { expectedRevision: _ignored, ...request } = configRequestBody(expectedRevision)
  return { revision, ...request }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}
