import { useCallback, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import { getProfile, patchProfile } from "../api/profile"
import type {
  PatchUserProfileRequest,
  UserProfile,
} from "../api/profile.generated"

export interface ProfileApi {
  getProfile: (signal?: AbortSignal) => Promise<UserProfile>
  patchProfile: (
    csrfToken: string,
    patch: PatchUserProfileRequest,
    signal?: AbortSignal,
  ) => Promise<UserProfile>
}

export type ProfileControllerError = "LOAD" | "SAVE"
export type ProfileFieldError = "INVALID" | "TAKEN"

export interface ProfileController {
  profile: UserProfile
  loadStatus: "idle" | "loading" | "ready" | "error"
  error: ProfileControllerError | null
  fieldErrors: Partial<Record<"displayName" | "email", ProfileFieldError>>
  isSaving: boolean
  load: () => Promise<void>
  cancel: () => void
  save: (draft: UserProfile) => Promise<boolean>
  clearError: () => void
}

interface UseProfileControllerOptions {
  csrfToken: string
  initialProfile: UserProfile
  onUnauthenticated: () => void
  api?: ProfileApi
}

const defaultApi: ProfileApi = { getProfile, patchProfile }

export function useProfileController({
  csrfToken,
  initialProfile,
  onUnauthenticated,
  api = defaultApi,
}: UseProfileControllerOptions): ProfileController {
  const [profile, setProfile] = useState(initialProfile)
  const [loadStatus, setLoadStatus] = useState<ProfileController["loadStatus"]>("idle")
  const [error, setError] = useState<ProfileControllerError | null>(null)
  const [fieldErrors, setFieldErrors] = useState<ProfileController["fieldErrors"]>({})
  const [isSaving, setIsSaving] = useState(false)
  const isSavingRef = useRef(false)
  const loadGeneration = useRef(0)
  const loadAbort = useRef<AbortController | null>(null)

  const clearError = useCallback(() => {
    setError(null)
    setFieldErrors({})
  }, [])

  const load = useCallback(async () => {
    loadAbort.current?.abort()
    const generation = ++loadGeneration.current
    const abort = new AbortController()
    loadAbort.current = abort
    setLoadStatus("loading")
    clearError()
    try {
      const loaded = await api.getProfile(abort.signal)
      if (generation !== loadGeneration.current) return
      setProfile(loaded)
      setLoadStatus("ready")
    } catch (cause) {
      if (generation !== loadGeneration.current || isAbortError(cause)) return
      if (isAuthenticationError(cause)) {
        onUnauthenticated()
        return
      }
      setLoadStatus("error")
      setError("LOAD")
    } finally {
      if (generation === loadGeneration.current) loadAbort.current = null
    }
  }, [api, clearError, onUnauthenticated])

  const cancel = useCallback(() => {
    loadGeneration.current += 1
    loadAbort.current?.abort()
    loadAbort.current = null
  }, [])

  const save = useCallback(async (draft: UserProfile) => {
    if (isSavingRef.current) return false
    const patch = profilePatch(profile, draft)
    if (!patch) {
      clearError()
      return true
    }
    isSavingRef.current = true
    setIsSaving(true)
    clearError()
    try {
      const persisted = await api.patchProfile(csrfToken, patch)
      setProfile(persisted)
      return true
    } catch (cause) {
      if (isAuthenticationError(cause)) {
        onUnauthenticated()
        return false
      }
      setError("SAVE")
      setFieldErrors(profileFieldErrors(cause))
      return false
    } finally {
      isSavingRef.current = false
      setIsSaving(false)
    }
  }, [api, clearError, csrfToken, onUnauthenticated, profile])

  return {
    profile,
    loadStatus,
    error,
    fieldErrors,
    isSaving,
    load,
    cancel,
    save,
    clearError,
  }
}

function profilePatch(
  current: UserProfile,
  draft: UserProfile,
): PatchUserProfileRequest | null {
  const patch: Partial<Pick<UserProfile, "displayName" | "email">> = {}
  if (current.displayName !== draft.displayName) patch.displayName = draft.displayName
  if (current.email !== draft.email) patch.email = draft.email
  return Object.keys(patch).length > 0
    ? (patch as PatchUserProfileRequest)
    : null
}

function profileFieldErrors(
  cause: unknown,
): ProfileController["fieldErrors"] {
  if (!(cause instanceof ApiClientError)) return {}
  const errors: ProfileController["fieldErrors"] = {}
  if (cause.payload.fields?.displayName) errors.displayName = "INVALID"
  if (cause.payload.fields?.email) {
    errors.email = cause.payload.code === "PROFILE_EMAIL_TAKEN" ? "TAKEN" : "INVALID"
  }
  return errors
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
