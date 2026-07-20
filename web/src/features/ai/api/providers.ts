import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isProvider,
  isProviderList,
  type CreateProviderRequest,
  type Provider,
  type ProviderList,
  type UpdateProviderRequest,
} from "./provider.generated"

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

function providerPath(providerId: string): string {
  return `${PROVIDERS_PATH}/${encodeURIComponent(providerId)}`
}
