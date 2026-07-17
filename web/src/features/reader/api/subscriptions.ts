import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isCreateSubscriptionResponse,
  isRefreshResponse,
  isSubscriptionPageResponse,
  isSubscriptionResponse,
  type CreateSubscriptionRequest,
  type CreateSubscriptionResponse,
  type RefreshResponse,
  type RefreshSubscriptionRequest,
  type SubscriptionPageResponse,
  type SubscriptionResponse,
} from "./subscription.generated"

export interface ListSubscriptionsOptions {
  cursor?: string
  limit?: number
  signal?: AbortSignal
}

export async function listSubscriptions(
  options: ListSubscriptionsOptions = {},
): Promise<SubscriptionPageResponse> {
  const query = new URLSearchParams()
  if (options.cursor !== undefined) query.set("cursor", options.cursor)
  if (options.limit !== undefined) query.set("limit", String(options.limit))
  const response = await apiRequest(withQuery("/api/v1/subscriptions", query), {
    signal: options.signal,
  })
  if (!isSubscriptionPageResponse(response)) throw invalidResponseError()
  return response
}

export async function getSubscription(
  subscriptionId: string,
  signal?: AbortSignal,
): Promise<SubscriptionResponse> {
  const response = await apiRequest(subscriptionPath(subscriptionId), { signal })
  if (!isSubscriptionResponse(response)) throw invalidResponseError()
  return response
}

export async function createSubscription(
  request: CreateSubscriptionRequest,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<CreateSubscriptionResponse> {
  const response = await apiRequest("/api/v1/subscriptions", {
    method: "POST",
    headers: csrfHeaders(csrfToken),
    body: JSON.stringify(request),
    signal,
  })
  if (!isCreateSubscriptionResponse(response)) throw invalidResponseError()
  return response
}

export async function deleteSubscription(
  subscriptionId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await apiRequest(subscriptionPath(subscriptionId), {
    method: "DELETE",
    headers: csrfHeaders(csrfToken),
    signal,
  })
  if (response !== undefined) throw invalidResponseError()
}

export async function refreshSubscription(
  subscriptionId: string,
  request: RefreshSubscriptionRequest,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<RefreshResponse> {
  const response = await apiRequest(`${subscriptionPath(subscriptionId)}/refresh`, {
    method: "POST",
    headers: csrfHeaders(csrfToken),
    body: JSON.stringify(request),
    signal,
  })
  if (!isRefreshResponse(response)) throw invalidResponseError()
  return response
}

function subscriptionPath(subscriptionId: string): string {
  return `/api/v1/subscriptions/${encodeURIComponent(subscriptionId)}`
}

function csrfHeaders(csrfToken: string): HeadersInit {
  return { "x-csrf-token": csrfToken }
}

function withQuery(path: string, query: URLSearchParams): string {
  const serialized = query.toString()
  return serialized.length === 0 ? path : `${path}?${serialized}`
}
