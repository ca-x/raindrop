import { ApiClientError, apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isEntryDetailResponse,
  isEntryPageResponse,
  isEntryStateResponse,
  type EntryDetailResponse,
  type EntryListState,
  type EntryPageResponse,
  type EntryStateResponse,
  type MarkEntriesReadRequest,
  type PatchEntryStateRequest,
} from "./reader.generated"

interface ListEntriesBaseOptions {
  cursor?: string
  limit?: number
  state?: EntryListState
  signal?: AbortSignal
}

export type ListEntriesOptions = ListEntriesBaseOptions &
  (
    | { feedId: string; categoryId?: never; search?: string }
    | { categoryId: string; feedId?: never; search?: never }
    | { feedId?: undefined; categoryId?: undefined; search?: never }
  )

export async function listEntries(
  options: ListEntriesOptions = {},
): Promise<EntryPageResponse> {
  const query = new URLSearchParams()
  if (options.cursor !== undefined) query.set("cursor", options.cursor)
  if (options.limit !== undefined) query.set("limit", String(options.limit))
  if (options.feedId !== undefined) query.set("feedId", options.feedId)
  if (options.categoryId !== undefined) query.set("categoryId", options.categoryId)
  if (options.search !== undefined) query.set("search", options.search)
  if (options.state !== undefined) query.set("state", options.state)
  const path = withQuery("/api/v1/entries", query)
  let response: unknown
  try {
    response = await apiRequest(path, { signal: options.signal })
  } catch (error) {
    const transient = error instanceof TypeError || (
      error instanceof ApiClientError &&
      [500, 502, 503, 504].includes(error.status) &&
      error.payload.code !== "INVALID_RESPONSE"
    )
    if (!transient || options.signal?.aborted) throw error
    // Only retry this idempotent read, once. Switching sources cancels the wait.
    await waitForReadRetry(options.signal)
    response = await apiRequest(path, { signal: options.signal })
  }
  if (!isEntryPageResponse(response)) throw invalidResponseError()
  return response
}

function waitForReadRetry(signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const abort = () => {
      clearTimeout(timer)
      reject(signal?.reason ?? new DOMException("Aborted", "AbortError"))
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", abort)
      resolve()
    }, 750)
    if (signal?.aborted) abort()
    else signal?.addEventListener("abort", abort, { once: true })
  })
}

export async function markEntriesRead(
  request: MarkEntriesReadRequest,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await apiRequest("/api/v1/entries/mark-read", {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (response !== undefined) throw invalidResponseError()
}

export async function getEntry(
  entryId: string,
  signal?: AbortSignal,
): Promise<EntryDetailResponse> {
  const response = await apiRequest(entryPath(entryId), { signal })
  if (!isEntryDetailResponse(response)) throw invalidResponseError()
  return response
}

export async function patchEntryState(
  entryId: string,
  request: PatchEntryStateRequest,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<EntryStateResponse> {
  const response = await apiRequest(`${entryPath(entryId)}/state`, {
    method: "PATCH",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isEntryStateResponse(response)) throw invalidResponseError()
  return response
}

function entryPath(entryId: string): string {
  return `/api/v1/entries/${encodeURIComponent(entryId)}`
}

function withQuery(path: string, query: URLSearchParams): string {
  const serialized = query.toString()
  return serialized.length === 0 ? path : `${path}?${serialized}`
}
