export type BootstrapStatus = "SETUP_REQUIRED" | "READY"

export interface BootstrapResponse {
  status: BootstrapStatus
  version: string
}

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

export type InitialAppState =
  | { phase: "setup"; bootstrap: BootstrapResponse }
  | { phase: "login"; bootstrap: BootstrapResponse }
  | { phase: "ready"; bootstrap: BootstrapResponse; session: SessionResponse }

export async function loadInitialAppState(signal: AbortSignal): Promise<InitialAppState> {
  const bootstrap = await getJson("/api/v1/bootstrap", signal)
  if (!isBootstrapResponse(bootstrap)) {
    throw new Error("invalid bootstrap response")
  }
  if (bootstrap.status === "SETUP_REQUIRED") {
    return { phase: "setup", bootstrap }
  }

  const sessionResponse = await fetch("/api/v1/auth/session", {
    credentials: "same-origin",
    signal,
  })
  if (sessionResponse.status === 401) {
    return { phase: "login", bootstrap }
  }
  if (!sessionResponse.ok) {
    throw new Error("session request failed")
  }
  const session: unknown = await sessionResponse.json()
  if (!isSessionResponse(session)) {
    throw new Error("invalid session response")
  }
  return { phase: "ready", bootstrap, session }
}

async function getJson(url: string, signal: AbortSignal): Promise<unknown> {
  const response = await fetch(url, { credentials: "same-origin", signal })
  if (!response.ok) {
    throw new Error("request failed")
  }
  return response.json()
}

function isBootstrapResponse(value: unknown): value is BootstrapResponse {
  if (!isRecord(value)) return false
  return (
    (value.status === "SETUP_REQUIRED" || value.status === "READY") &&
    typeof value.version === "string"
  )
}

function isSessionResponse(value: unknown): value is SessionResponse {
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
