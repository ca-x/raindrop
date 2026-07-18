import { isSessionResponse, type SessionResponse } from "../features/auth/session"

export type BootstrapStatus = "SETUP_REQUIRED" | "READY"
export type SetupMode = "FULL" | "ADMIN_ONLY"

export interface BootstrapResponse {
  status: BootstrapStatus
  version: string
  setupMode?: SetupMode
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
  if (typeof value.version !== "string") return false
  if (value.status === "READY") return value.setupMode === undefined
  return (
    value.status === "SETUP_REQUIRED" &&
    (value.setupMode === "FULL" || value.setupMode === "ADMIN_ONLY")
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}
