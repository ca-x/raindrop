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
  if (!isRecord(value) || !isRecord(value.user)) return false
  return (
    typeof value.csrfToken === "string" &&
    typeof value.expiresAt === "string" &&
    typeof value.user.id === "string" &&
    typeof value.user.username === "string" &&
    (typeof value.user.email === "string" || value.user.email === null) &&
    typeof value.user.isDisabled === "boolean" &&
    Array.isArray(value.user.roles) &&
    value.user.roles.every((role) => typeof role === "string")
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}
