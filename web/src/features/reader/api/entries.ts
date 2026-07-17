import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isEntryDetailResponse,
  isEntryPageResponse,
  isEntryStateResponse,
  type EntryDetailResponse,
  type EntryListState,
  type EntryPageResponse,
  type EntryStateResponse,
  type PatchEntryStateRequest,
} from "./reader.generated"

export interface ListEntriesOptions {
  cursor?: string
  limit?: number
  feedId?: string
  state?: EntryListState
  signal?: AbortSignal
}

export async function listEntries(
  options: ListEntriesOptions = {},
): Promise<EntryPageResponse> {
  const query = new URLSearchParams()
  if (options.cursor !== undefined) query.set("cursor", options.cursor)
  if (options.limit !== undefined) query.set("limit", String(options.limit))
  if (options.feedId !== undefined) query.set("feedId", options.feedId)
  if (options.state !== undefined) query.set("state", options.state)
  const response = await apiRequest(withQuery("/api/v1/entries", query), {
    signal: options.signal,
  })
  if (!isEntryPageResponse(response)) throw invalidResponseError()
  return response
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
