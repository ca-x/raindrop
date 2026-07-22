import {
  ApiClientError,
  apiRequest,
  apiResponse,
  invalidResponseError,
} from "../../../shared/api/client"
import {
  isLookupResult,
  isTranslationConfig,
  isTranslationProgressEvent,
  isTranslationResult,
  isTranslationTestResult,
  isTranslationTextResult,
  type LookupResult,
  type PutTranslationConfigRequest,
  type TestTranslationRequest,
  type TranslationConfig,
  type TranslationProgressEvent,
  type TranslationResult,
  type TranslationTestResult,
  type TranslationTextResult,
} from "./translation.generated"

const TRANSLATION_PATH = "/api/v3/plugins/translation"
const MAX_PROGRESS_LINE_CHARACTERS = 100_000

export async function getTranslationConfig(
  signal?: AbortSignal,
): Promise<TranslationConfig> {
  const response = await apiRequest(TRANSLATION_PATH, { signal })
  if (!isTranslationConfig(response)) throw invalidResponseError()
  return response
}

export async function putTranslationConfig(
  csrfToken: string,
  request: PutTranslationConfigRequest,
  signal?: AbortSignal,
): Promise<TranslationConfig> {
  const response = await apiRequest(TRANSLATION_PATH, {
    method: "PUT",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isTranslationConfig(response)) throw invalidResponseError()
  return response
}

export async function testTranslationConnection(
  csrfToken: string,
  request: TestTranslationRequest,
  signal?: AbortSignal,
): Promise<TranslationTestResult> {
  const response = await apiRequest(`${TRANSLATION_PATH}/test`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isTranslationTestResult(response)) throw invalidResponseError()
  return response
}

export async function translateEntry(
  entryId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<TranslationResult> {
  const response = await apiRequest(
    `${TRANSLATION_PATH}/entries/${encodeURIComponent(entryId)}/translate`,
    {
      method: "POST",
      headers: { "x-csrf-token": csrfToken },
      signal,
    },
  )
  if (!isTranslationResult(response)) throw invalidResponseError()
  return response
}

export async function translateEntryProgressively(
  entryId: string,
  csrfToken: string,
  onProgress: (event: TranslationProgressEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  const response = await apiResponse(
    `${TRANSLATION_PATH}/entries/${encodeURIComponent(entryId)}/translate/progressive`,
    {
      method: "POST",
      headers: { "x-csrf-token": csrfToken },
      signal,
    },
  )
  const contentType = response.headers.get("content-type")?.split(";", 1)[0]?.trim()
  if (contentType !== "application/x-ndjson" || !response.body) {
    throw invalidResponseError()
  }
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ""
  let started = false
  let titleReceived = false
  let completed = false
  let totalSegments = 0
  let completedSegments = 0

  const consumeLine = (line: string) => {
    if (!line.trim()) return
    if (line.length > MAX_PROGRESS_LINE_CHARACTERS) throw invalidResponseError()
    let value: unknown
    try {
      value = JSON.parse(line)
    } catch {
      throw invalidResponseError()
    }
    if (!isTranslationProgressEvent(value)) throw invalidResponseError()
    switch (value.kind) {
      case "STARTED":
        if (
          started ||
          value.totalSegments < 1 ||
          value.targetLocale === null ||
          value.error !== null
        ) {
          throw invalidResponseError()
        }
        started = true
        totalSegments = value.totalSegments
        break
      case "TITLE":
        if (
          !started ||
          titleReceived ||
          value.title === null ||
          value.providerLabel === null ||
          value.targetLocale === null ||
          value.totalSegments !== totalSegments ||
          value.error !== null
        ) {
          throw invalidResponseError()
        }
        titleReceived = true
        break
      case "SEGMENT":
        if (
          !titleReceived ||
          value.segment === null ||
          value.segment.index !== completedSegments ||
          value.completedSegments !== completedSegments + 1 ||
          value.completedSegments > totalSegments ||
          value.totalSegments !== totalSegments ||
          value.error !== null
        ) {
          throw invalidResponseError()
        }
        completedSegments = value.completedSegments
        break
      case "COMPLETED":
        if (
          !titleReceived ||
          completed ||
          completedSegments !== totalSegments ||
          value.completedSegments !== totalSegments ||
          value.totalSegments !== totalSegments ||
          value.error !== null
        ) {
          throw invalidResponseError()
        }
        completed = true
        break
      case "ERROR":
        if (value.error === null) throw invalidResponseError()
        throw new ApiClientError(value.error.status, {
          code: value.error.code,
          message: value.error.message,
        })
    }
    onProgress(value)
  }

  while (true) {
    const { done, value } = await reader.read()
    buffer += decoder.decode(value, { stream: !done })
    if (buffer.length > MAX_PROGRESS_LINE_CHARACTERS * 2) {
      throw invalidResponseError()
    }
    let newline = buffer.indexOf("\n")
    while (newline >= 0) {
      consumeLine(buffer.slice(0, newline))
      buffer = buffer.slice(newline + 1)
      newline = buffer.indexOf("\n")
    }
    if (done) break
  }
  consumeLine(buffer)
  if (!completed) throw invalidResponseError()
}

export async function translateSelectedText(
  text: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<TranslationTextResult> {
  const response = await apiRequest(`${TRANSLATION_PATH}/translate`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify({ text }),
    signal,
  })
  if (!isTranslationTextResult(response)) throw invalidResponseError()
  return response
}

export async function lookupTranslation(
  text: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<LookupResult> {
  const response = await apiRequest(`${TRANSLATION_PATH}/lookup`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify({ text }),
    signal,
  })
  if (!isLookupResult(response)) throw invalidResponseError()
  return response
}
