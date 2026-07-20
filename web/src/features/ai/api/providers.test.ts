import { afterEach, expect, it, vi } from "vitest"

import type {
  CreateProviderRequest,
  Provider,
  UpdateProviderRequest,
} from "./provider.generated"
import {
  createProvider,
  getProvider,
  listProviders,
  updateProvider,
} from "./providers"

afterEach(() => vi.unstubAllGlobals())

const providerId = "00000000-0000-4000-8000-000000000101"
const provider: Provider = {
  providerId,
  scope: "USER",
  canEdit: true,
  displayName: "Primary model",
  kind: "OPENAI_RESPONSES",
  endpoint: "https://api.openai.com/v1/responses",
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

const createRequest: CreateProviderRequest = {
  displayName: provider.displayName,
  kind: provider.kind,
  endpoint: null,
  model: provider.model,
  credential: "credential-kept-in-memory-only",
  capabilities: provider.capabilities,
  policy: provider.policy,
  isEnabled: true,
}

it("lists and gets providers through generated validators", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse({ keyringStatus: "AVAILABLE", items: [provider] }))
    .mockResolvedValueOnce(jsonResponse(provider))
  vi.stubGlobal("fetch", fetchMock)

  await expect(listProviders()).resolves.toEqual({
    keyringStatus: "AVAILABLE",
    items: [provider],
  })
  await expect(getProvider(providerId)).resolves.toEqual(provider)

  expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/v1/ai/providers")
  expect(fetchMock.mock.calls[1]?.[0]).toBe(`/api/v1/ai/providers/${providerId}`)
})

it("creates a provider with CSRF, AbortSignal, and no credential readback", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(provider, 201))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  const result = await createProvider("csrf-memory", createRequest, signal)

  expect(result).toEqual(provider)
  expect(result).not.toHaveProperty("credential")
  const [path, init] = fetchMock.mock.calls[0] ?? []
  expect(path).toBe("/api/v1/ai/providers")
  expect(init?.method).toBe("POST")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual(createRequest)
})

it("patches one provider with its exact revision and an optional credential", async () => {
  const request: UpdateProviderRequest = {
    expectedRevision: 0,
    displayName: "Updated model",
  }
  const updated = { ...provider, displayName: request.displayName!, revision: 1 }
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(updated))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await expect(
    updateProvider(providerId, "csrf-memory", request, signal),
  ).resolves.toEqual(updated)

  const [path, init] = fetchMock.mock.calls[0] ?? []
  expect(path).toBe(`/api/v1/ai/providers/${providerId}`)
  expect(init?.method).toBe("PATCH")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual(request)
})

it.each([
  ["list", () => listProviders(), { keyringStatus: "AVAILABLE", items: [{ ...provider, credential: "leak" }] }],
  ["get", () => getProvider(providerId), { ...provider, encryptedSecret: "leak" }],
  ["create", () => createProvider("csrf", createRequest), { ...provider, revision: -1 }],
  [
    "patch",
    () => updateProvider(providerId, "csrf", { expectedRevision: 0, isEnabled: false }),
    { ...provider, scope: "ADMIN" },
  ],
])("rejects a malformed %s success response", async (_name, request, body) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(body)))
  await expect(request()).rejects.toMatchObject({
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("does not copy the submitted credential into validation errors", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ ...provider, revision: -1 }, 201)))

  const error = await createProvider("csrf", createRequest).catch((cause: unknown) => cause)

  expect(String(error)).not.toContain(createRequest.credential)
  expect(JSON.stringify(error)).not.toContain(createRequest.credential)
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
