import { expect, it } from "vitest"

import type { Provider } from "../api/provider.generated"
import {
  changeProviderKind,
  createProviderDraft,
  editProviderDraft,
  providerDraftRequest,
  validateProviderDraft,
} from "./providerDraft"

const provider: Provider = {
  providerId: "00000000-0000-4000-8000-000000000101",
  scope: "USER",
  canEdit: true,
  displayName: "Primary model",
  kind: "OPENAI_RESPONSES",
  endpoint: "https://proxy.example.com/openai/",
  model: "gpt-5-mini",
  capabilities: { supportsUsage: true, supportsIdempotency: true },
  policy: {
    maxConcurrency: 3,
    requestsPerMinute: 80,
    maxInputTokensPerRequest: 65_536,
    maxOutputTokensPerRequest: 8192,
    inputCostMicrosPerMillionTokens: 1_250_000,
    outputCostMicrosPerMillionTokens: 10_000_000,
    maxCostMicrosPerRequest: 300_000,
  },
  isEnabled: true,
  revision: 4,
  createdAt: "2026-07-20T10:00:00Z",
  updatedAt: "2026-07-20T10:00:00Z",
}

it("provides kind-specific endpoint and capability defaults with bounded policy", () => {
  const openAi = createProviderDraft("OPENAI_RESPONSES")
  expect(openAi.endpoint).toBe("https://api.openai.com/")
  expect(openAi.capabilities).toEqual({
    supportsUsage: true,
    supportsIdempotency: true,
  })
  expect(openAi.policy).toEqual({
    maxConcurrency: 2,
    requestsPerMinute: 60,
    maxInputTokensPerRequest: 32_768,
    maxOutputTokensPerRequest: 4096,
    inputCostMicrosPerMillionTokens: null,
    outputCostMicrosPerMillionTokens: null,
    maxCostMicrosPerRequest: 250_000,
  })

  const gemini = changeProviderKind(openAi, "GOOGLE_GEMINI")
  expect(gemini.endpoint).toBe("https://generativelanguage.googleapis.com/")
  expect(gemini.capabilities.supportsIdempotency).toBe(false)

  const custom = changeProviderKind(
    { ...gemini, endpoint: "https://gateway.example.com/gemini/" },
    "ANTHROPIC_MESSAGES",
  )
  expect(custom.endpoint).toBe("https://gateway.example.com/gemini/")
})

it("requires identity, model, endpoint, and credential when creating", () => {
  const draft = createProviderDraft()
  expect(validateProviderDraft(draft)).toEqual({
    displayName: "REQUIRED",
    model: "REQUIRED",
    credential: "REQUIRED",
  })

  const invalid = validateProviderDraft({
    ...draft,
    displayName: "x".repeat(81),
    model: "model",
    credential: "secret",
    endpoint: "http://localhost:8080/",
  })
  expect(invalid).toMatchObject({ displayName: "TOO_LONG", endpoint: "HTTPS" })
})

it("validates exact numeric policy ranges", () => {
  const draft = {
    ...createProviderDraft(),
    displayName: "Model",
    model: "model",
    credential: "secret",
    policy: {
      maxConcurrency: 0,
      requestsPerMinute: 1_000_001,
      maxInputTokensPerRequest: 1_048_577,
      maxOutputTokensPerRequest: 16_385,
      inputCostMicrosPerMillionTokens: -1,
      outputCostMicrosPerMillionTokens: 1_000_000_000_001,
      maxCostMicrosPerRequest: -1,
    },
  }

  expect(validateProviderDraft(draft)).toMatchObject({
    maxConcurrency: "RANGE",
    requestsPerMinute: "RANGE",
    maxInputTokensPerRequest: "RANGE",
    maxOutputTokensPerRequest: "RANGE",
    inputCostMicrosPerMillionTokens: "RANGE",
    outputCostMicrosPerMillionTokens: "RANGE",
    maxCostMicrosPerRequest: "RANGE",
  })
})

it("keeps an edit credential blank and omits unchanged secret rotation", () => {
  const draft = editProviderDraft(provider)
  expect(draft.credential).toBe("")

  const request = providerDraftRequest(draft)
  expect(request.ok).toBe(true)
  if (!request.ok || request.mode !== "edit") return
  expect(request.providerId).toBe(provider.providerId)
  expect(request.request.expectedRevision).toBe(4)
  expect(request.request).not.toHaveProperty("credential")

  const rotated = providerDraftRequest({ ...draft, credential: "replacement" })
  expect(rotated.ok).toBe(true)
  if (!rotated.ok || rotated.mode !== "edit") return
  expect(rotated.request.credential).toBe("replacement")
})

it("normalizes whitespace without retaining a create credential in output state", () => {
  const result = providerDraftRequest({
    ...createProviderDraft(),
    displayName: "  Primary model  ",
    model: "  gpt-5-mini  ",
    credential: "secret-value",
  })
  expect(result.ok).toBe(true)
  if (!result.ok || result.mode !== "create") return
  expect(result.request.displayName).toBe("Primary model")
  expect(result.request.model).toBe("gpt-5-mini")
  expect(result.request.credential).toBe("secret-value")
})
