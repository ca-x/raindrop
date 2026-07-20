import type {
  CreateProviderRequest,
  Provider,
  ProviderCapabilities,
  ProviderKind,
  ProviderPolicy,
  UpdateProviderRequest,
} from "../api/provider.generated"

export interface ProviderDraft {
  mode: "create" | "edit"
  providerId: string | null
  expectedRevision: number | null
  displayName: string
  kind: ProviderKind
  endpoint: string
  model: string
  credential: string
  capabilities: ProviderCapabilities
  policy: ProviderPolicy
  isEnabled: boolean
}

export type ProviderDraftField =
  | "displayName"
  | "endpoint"
  | "model"
  | "credential"
  | keyof ProviderPolicy

export type ProviderDraftError = "REQUIRED" | "TOO_LONG" | "HTTPS" | "RANGE"
export type ProviderDraftErrors = Partial<
  Record<ProviderDraftField, ProviderDraftError>
>

export type ProviderDraftRequestResult =
  | { ok: false; errors: ProviderDraftErrors }
  | { ok: true; mode: "create"; request: CreateProviderRequest }
  | {
      ok: true
      mode: "edit"
      providerId: string
      request: UpdateProviderRequest
    }

const DEFAULT_POLICY: ProviderPolicy = {
  maxConcurrency: 2,
  requestsPerMinute: 60,
  maxInputTokensPerRequest: 32_768,
  maxOutputTokensPerRequest: 4096,
  inputCostMicrosPerMillionTokens: null,
  outputCostMicrosPerMillionTokens: null,
  maxCostMicrosPerRequest: 250_000,
}

const KIND_DEFAULTS: Record<
  ProviderKind,
  { endpoint: string; capabilities: ProviderCapabilities }
> = {
  ANTHROPIC_MESSAGES: {
    endpoint: "https://api.anthropic.com/",
    capabilities: { supportsUsage: true, supportsIdempotency: false },
  },
  OPENAI_RESPONSES: {
    endpoint: "https://api.openai.com/",
    capabilities: { supportsUsage: true, supportsIdempotency: true },
  },
  OPENAI_CHAT_COMPLETIONS: {
    endpoint: "https://api.openai.com/",
    capabilities: { supportsUsage: true, supportsIdempotency: true },
  },
  GOOGLE_GEMINI: {
    endpoint: "https://generativelanguage.googleapis.com/",
    capabilities: { supportsUsage: true, supportsIdempotency: false },
  },
}

export function createProviderDraft(
  kind: ProviderKind = "OPENAI_RESPONSES",
): ProviderDraft {
  const defaults = KIND_DEFAULTS[kind]
  return {
    mode: "create",
    providerId: null,
    expectedRevision: null,
    displayName: "",
    kind,
    endpoint: defaults.endpoint,
    model: "",
    credential: "",
    capabilities: { ...defaults.capabilities },
    policy: { ...DEFAULT_POLICY },
    isEnabled: true,
  }
}

export function editProviderDraft(provider: Provider): ProviderDraft {
  return {
    mode: "edit",
    providerId: provider.providerId,
    expectedRevision: provider.revision,
    displayName: provider.displayName,
    kind: provider.kind,
    endpoint: provider.endpoint,
    model: provider.model,
    credential: "",
    capabilities: { ...provider.capabilities },
    policy: { ...provider.policy },
    isEnabled: provider.isEnabled,
  }
}

export function changeProviderKind(
  draft: ProviderDraft,
  kind: ProviderKind,
): ProviderDraft {
  const previousDefault = KIND_DEFAULTS[draft.kind].endpoint
  const next = KIND_DEFAULTS[kind]
  return {
    ...draft,
    kind,
    endpoint:
      draft.endpoint.trim().length === 0 || draft.endpoint === previousDefault
        ? next.endpoint
        : draft.endpoint,
    capabilities: { ...next.capabilities },
  }
}

export function validateProviderDraft(draft: ProviderDraft): ProviderDraftErrors {
  const errors: ProviderDraftErrors = {}
  const displayName = draft.displayName.trim()
  const endpoint = draft.endpoint.trim()
  const model = draft.model.trim()

  if (displayName.length === 0) errors.displayName = "REQUIRED"
  else if (byteLength(displayName) > 80) errors.displayName = "TOO_LONG"
  if (model.length === 0) errors.model = "REQUIRED"
  else if (byteLength(model) > 200) errors.model = "TOO_LONG"
  if (endpoint.length === 0 || !isSafeHttpsEndpoint(endpoint)) {
    errors.endpoint = "HTTPS"
  } else if (byteLength(endpoint) > 2048) {
    errors.endpoint = "TOO_LONG"
  }
  if (draft.mode === "create" && draft.credential.length === 0) {
    errors.credential = "REQUIRED"
  } else if (byteLength(draft.credential) > 8192) {
    errors.credential = "TOO_LONG"
  }

  validateInteger(errors, "maxConcurrency", draft.policy.maxConcurrency, 1, 64)
  validateNullableInteger(
    errors,
    "requestsPerMinute",
    draft.policy.requestsPerMinute,
    1,
    1_000_000,
  )
  validateInteger(
    errors,
    "maxInputTokensPerRequest",
    draft.policy.maxInputTokensPerRequest,
    1,
    1_048_576,
  )
  validateInteger(
    errors,
    "maxOutputTokensPerRequest",
    draft.policy.maxOutputTokensPerRequest,
    1,
    16_384,
  )
  validateNullableInteger(
    errors,
    "inputCostMicrosPerMillionTokens",
    draft.policy.inputCostMicrosPerMillionTokens,
    0,
    1_000_000_000_000,
  )
  validateNullableInteger(
    errors,
    "outputCostMicrosPerMillionTokens",
    draft.policy.outputCostMicrosPerMillionTokens,
    0,
    1_000_000_000_000,
  )
  validateNullableInteger(
    errors,
    "maxCostMicrosPerRequest",
    draft.policy.maxCostMicrosPerRequest,
    0,
    1_000_000_000_000,
  )
  return errors
}

export function providerDraftRequest(
  draft: ProviderDraft,
): ProviderDraftRequestResult {
  const errors = validateProviderDraft(draft)
  if (Object.keys(errors).length > 0) return { ok: false, errors }

  const shared = {
    displayName: draft.displayName.trim(),
    endpoint: draft.endpoint.trim(),
    model: draft.model.trim(),
    capabilities: { ...draft.capabilities },
    policy: { ...draft.policy },
    isEnabled: draft.isEnabled,
  }
  if (draft.mode === "create") {
    return {
      ok: true,
      mode: "create",
      request: {
        ...shared,
        kind: draft.kind,
        credential: draft.credential,
      },
    }
  }

  if (draft.providerId === null || draft.expectedRevision === null) {
    return { ok: false, errors: { displayName: "REQUIRED" } }
  }
  return {
    ok: true,
    mode: "edit",
    providerId: draft.providerId,
    request: {
      expectedRevision: draft.expectedRevision,
      ...shared,
      ...(draft.credential.length > 0 ? { credential: draft.credential } : {}),
    },
  }
}

function isSafeHttpsEndpoint(value: string): boolean {
  try {
    const url = new URL(value)
    return (
      url.protocol === "https:" &&
      url.hostname.length > 0 &&
      url.username.length === 0 &&
      url.password.length === 0 &&
      url.search.length === 0 &&
      url.hash.length === 0
    )
  } catch {
    return false
  }
}

function validateInteger(
  errors: ProviderDraftErrors,
  field: keyof ProviderPolicy,
  value: number,
  minimum: number,
  maximum: number,
) {
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    errors[field] = "RANGE"
  }
}

function validateNullableInteger(
  errors: ProviderDraftErrors,
  field: keyof ProviderPolicy,
  value: number | null,
  minimum: number,
  maximum: number,
) {
  if (value !== null) validateInteger(errors, field, value, minimum, maximum)
}

function byteLength(value: string): number {
  return new TextEncoder().encode(value).length
}
