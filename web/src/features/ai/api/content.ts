import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isAiArtifact,
  isAiConfigEnvelope,
  isAiJob,
  isEntryAiOverview,
  type AiArtifact,
  type AiConfigEnvelope,
  type AiJob,
  type EnqueueAiJobRequest,
  type EntryAiOverview,
  type PutAiConfigRequest,
  type RetryAiJobRequest,
} from "./content.generated"

const CONFIG_PATH = "/api/v1/ai/config"

export async function getAiConfig(signal?: AbortSignal): Promise<AiConfigEnvelope> {
  const response = await apiRequest(CONFIG_PATH, { signal })
  if (!isAiConfigEnvelope(response)) throw invalidResponseError()
  return response
}

export async function putAiConfig(
  csrfToken: string,
  request: PutAiConfigRequest,
  signal?: AbortSignal,
): Promise<AiConfigEnvelope> {
  const response = await apiRequest(CONFIG_PATH, {
    method: "PUT",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isAiConfigEnvelope(response)) throw invalidResponseError()
  return response
}

export async function getEntryAiOverview(
  entryId: string,
  translationLocale?: string,
  signal?: AbortSignal,
): Promise<EntryAiOverview> {
  const query = new URLSearchParams()
  if (translationLocale !== undefined) {
    query.set("translationLocale", translationLocale)
  }
  const path = withQuery(`/api/v1/entries/${encodeURIComponent(entryId)}/ai`, query)
  const response = await apiRequest(path, { signal })
  if (!isEntryAiOverview(response)) throw invalidResponseError()
  return response
}

export async function enqueueAiJob(
  entryId: string,
  csrfToken: string,
  request: EnqueueAiJobRequest,
  signal?: AbortSignal,
): Promise<AiJob> {
  const response = await apiRequest(
    `/api/v1/entries/${encodeURIComponent(entryId)}/ai/jobs`,
    {
      method: "POST",
      headers: { "x-csrf-token": csrfToken },
      body: JSON.stringify(request),
      signal,
    },
  )
  if (!isAiJob(response)) throw invalidResponseError()
  return response
}

export async function getAiJob(
  jobId: string,
  signal?: AbortSignal,
): Promise<AiJob> {
  const response = await apiRequest(jobPath(jobId), { signal })
  if (!isAiJob(response)) throw invalidResponseError()
  return response
}

export async function getAiJobResult(
  jobId: string,
  signal?: AbortSignal,
): Promise<AiArtifact> {
  const response = await apiRequest(`${jobPath(jobId)}/result`, { signal })
  if (!isAiArtifact(response)) throw invalidResponseError()
  return response
}

export async function retryAiJob(
  jobId: string,
  csrfToken: string,
  request: RetryAiJobRequest,
  signal?: AbortSignal,
): Promise<AiJob> {
  const response = await apiRequest(`${jobPath(jobId)}/retry`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isAiJob(response)) throw invalidResponseError()
  return response
}

function jobPath(jobId: string): string {
  return `/api/v1/ai/jobs/${encodeURIComponent(jobId)}`
}

function withQuery(path: string, query: URLSearchParams): string {
  const serialized = query.toString()
  return serialized.length === 0 ? path : `${path}?${serialized}`
}
