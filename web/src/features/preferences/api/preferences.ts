import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isUserPreferences,
  type PatchUserPreferencesRequest,
  type UserPreferences,
} from "./preferences.generated"

const PREFERENCES_PATH = "/api/v1/preferences"

export async function getPreferences(signal?: AbortSignal): Promise<UserPreferences> {
  const response = await apiRequest(PREFERENCES_PATH, { signal })
  if (!isUserPreferences(response)) throw invalidResponseError()
  return response
}

export async function patchPreferences(
  csrfToken: string,
  patch: PatchUserPreferencesRequest,
  signal?: AbortSignal,
): Promise<UserPreferences> {
  const response = await apiRequest(PREFERENCES_PATH, {
    method: "PATCH",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(patch),
    signal,
  })
  if (!isUserPreferences(response)) throw invalidResponseError()
  return response
}
