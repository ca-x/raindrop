export interface SessionUser {
  id: string
  username: string
  email: string | null
  isDisabled: boolean
  roles: string[]
}

export interface SessionResponse {
  user: SessionUser
  csrfToken: string
  expiresAt: string
}

export function isSessionResponse(value: unknown): value is SessionResponse {
  if (!isRecord(value) || !isSessionUser(value.user)) return false
  return (
    typeof value.csrfToken === "string" &&
    typeof value.expiresAt === "string" &&
    value.csrfToken.length > 0 &&
    value.expiresAt.length > 0
  )
}

export function isSessionUser(value: unknown): value is SessionUser {
  if (!isRecord(value)) return false
  return (
    typeof value.id === "string" &&
    typeof value.username === "string" &&
    (typeof value.email === "string" || value.email === null) &&
    typeof value.isDisabled === "boolean" &&
    Array.isArray(value.roles) &&
    value.roles.every((role) => typeof role === "string")
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}
