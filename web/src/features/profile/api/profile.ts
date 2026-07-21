import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isUserProfile,
  type PatchUserProfileRequest,
  type UserProfile,
} from "./profile.generated"

const PROFILE_PATH = "/api/v2/profile"

export async function getProfile(signal?: AbortSignal): Promise<UserProfile> {
  const response = await apiRequest(PROFILE_PATH, { signal })
  if (!isUserProfile(response)) throw invalidResponseError()
  return response
}

export async function patchProfile(
  csrfToken: string,
  patch: PatchUserProfileRequest,
  signal?: AbortSignal,
): Promise<UserProfile> {
  const response = await apiRequest(PROFILE_PATH, {
    method: "PATCH",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(patch),
    signal,
  })
  if (!isUserProfile(response)) throw invalidResponseError()
  return response
}
