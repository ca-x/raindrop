import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isProvider,
  isProviderList,
  type CreateProviderRequest,
  type Provider,
  type ProviderKind,
  type ProviderList,
  type UpdateProviderRequest,
} from "./provider.generated"

export interface ProviderModelDiscoveryRequest {
  providerId?: string | null
  kind: ProviderKind
  endpoint: string
  credential: string
}

export type ProviderModelDiscoveryResult = { id: string; label: string }

const PROVIDERS_PATH = "/api/v1/ai/providers"

export async function listProviders(signal?: AbortSignal): Promise<ProviderList> {
  const response = await apiRequest(PROVIDERS_PATH, { signal })
  if (!isProviderList(response)) throw invalidResponseError()
  return response
}

export async function createProvider(
  csrfToken: string,
  request: CreateProviderRequest,
  signal?: AbortSignal,
): Promise<Provider> {
  const response = await apiRequest(PROVIDERS_PATH, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isProvider(response)) throw invalidResponseError()
  return response
}

export async function getProvider(
  providerId: string,
  signal?: AbortSignal,
): Promise<Provider> {
  const response = await apiRequest(providerPath(providerId), { signal })
  if (!isProvider(response)) throw invalidResponseError()
  return response
}

export async function updateProvider(
  providerId: string,
  csrfToken: string,
  request: UpdateProviderRequest,
  signal?: AbortSignal,
): Promise<Provider> {
  const response = await apiRequest(providerPath(providerId), {
    method: "PATCH",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isProvider(response)) throw invalidResponseError()
  return response
}

/**
 * Fetches the model catalog exposed by a provider's standard discovery endpoint.
 * This request is intentionally kept in-memory: credentials are sent only to
 * the provider over HTTPS and are never persisted or returned by Raindrop.
 */
export async function discoverProviderModels(
  csrfToken: string,
  request: ProviderModelDiscoveryRequest,
  signal?: AbortSignal,
): Promise<ProviderModelDiscoveryResult[]> {
  const credential = request.credential.trim()
  if (!credential) throw new Error("MODEL_DISCOVERY_CREDENTIAL_REQUIRED")
  const response = await apiRequest("/api/v1/ai/providers/models", {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify({
      providerId: request.providerId ?? undefined,
      kind: request.kind,
      endpoint: request.endpoint,
      credential,
    }),
    signal,
  })
  return parseModelDiscoveryResponse(response)
}

function parseModelDiscoveryResponse(payload: unknown): ProviderModelDiscoveryResult[] {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    throw new Error("MODEL_DISCOVERY_RESPONSE_INVALID")
  }
  const record = payload as Record<string, unknown>
  const values = Array.isArray(record.data)
    ? record.data
    : Array.isArray(record.models)
      ? record.models
      : []
  const models = values
    .map((value): ProviderModelDiscoveryResult | null => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return null
      const item = value as Record<string, unknown>
      const raw = typeof item.id === "string" ? item.id : typeof item.name === "string" ? item.name : null
      if (!raw) return null
      const id = raw.replace(/^models\//u, "").trim()
      if (!id || id.length > 200 || /[\u0000-\u001f\u007f]/u.test(id)) return null
      return { id, label: id }
    })
    .filter((value): value is ProviderModelDiscoveryResult => value !== null)
  const unique = new Map(models.map((model) => [model.id, model]))
  return [...unique.values()].sort((left, right) => left.label.localeCompare(right.label)).slice(0, 200)
}

function providerPath(providerId: string): string {
  return `${PROVIDERS_PATH}/${encodeURIComponent(providerId)}`
}
