import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isLookupResult,
  isTranslationConfig,
  isTranslationResult,
  isTranslationTestResult,
  isTranslationTextResult,
  type LookupResult,
  type PutTranslationConfigRequest,
  type TestTranslationRequest,
  type TranslationConfig,
  type TranslationResult,
  type TranslationTestResult,
  type TranslationTextResult,
} from "./translation.generated"

const TRANSLATION_PATH = "/api/v2/plugins/translation"

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
