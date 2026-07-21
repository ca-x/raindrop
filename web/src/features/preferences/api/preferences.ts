import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isUserFont,
  isUserFontList,
  isUserPreferences,
  type PatchUserPreferencesRequest,
  type UserFont,
  type UserFontList,
  type UserPreferences,
} from "./preferences.generated"

const PREFERENCES_PATH = "/api/v2/preferences"

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

export async function listUserFonts(signal?: AbortSignal): Promise<UserFontList> {
  const response = await apiRequest(`${PREFERENCES_PATH}/fonts`, { signal })
  if (!isUserFontList(response)) throw invalidResponseError()
  return response
}

export async function uploadUserFont(
  file: File,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<UserFont> {
  const name = file.name.replace(/\.woff2$/iu, "").trim() || file.name
  const query = new URLSearchParams({ name })
  const response = await apiRequest(`${PREFERENCES_PATH}/fonts?${query.toString()}`, {
    method: "POST",
    headers: {
      "content-type": file.type || "font/woff2",
      "x-csrf-token": csrfToken,
    },
    body: file,
    signal,
  })
  if (!isUserFont(response)) throw invalidResponseError()
  return response
}

export async function deleteUserFont(
  fontId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await apiRequest(
    `${PREFERENCES_PATH}/fonts/${encodeURIComponent(fontId)}`,
    {
      method: "DELETE",
      headers: { "x-csrf-token": csrfToken },
      signal,
    },
  )
  if (response !== undefined) throw invalidResponseError()
}
