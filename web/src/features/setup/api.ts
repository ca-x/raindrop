import { apiRequest, invalidResponseError } from "../../shared/api/client"
import { isSessionUser, type SessionUser } from "../auth/session"
import type { SetupValues } from "./model"

interface DatabaseCheckResponse {
  status: "OK"
  databaseKind: "SQLITE" | "POSTGRESQL" | "MYSQL"
}

interface SetupCompleteResponse {
  status: "READY"
  user: SessionUser
}

export async function checkDatabase(values: SetupValues): Promise<DatabaseCheckResponse> {
  const response = await apiRequest("/api/v1/setup/database-check", {
    method: "POST",
    headers: { "x-setup-token": values.token },
    body: JSON.stringify({ databaseUrl: values.databaseUrl }),
  })
  if (!isDatabaseCheckResponse(response)) throw invalidResponseError()
  return response
}

export async function completeSetup(values: SetupValues): Promise<SetupCompleteResponse> {
  const response = await apiRequest("/api/v1/setup/complete", {
    method: "POST",
    headers: { "x-setup-token": values.token },
    body: JSON.stringify({
      databaseUrl: values.databaseUrl,
      username: values.username,
      password: values.password,
      email: values.email || null,
    }),
  })
  if (!isSetupCompleteResponse(response)) throw invalidResponseError()
  return response
}

export async function completeAdminSetup(
  values: SetupValues,
): Promise<SetupCompleteResponse> {
  const response = await apiRequest("/api/v1/setup/admin", {
    method: "POST",
    headers: { "x-setup-token": values.token },
    body: JSON.stringify({
      username: values.username,
      password: values.password,
      email: values.email || null,
    }),
  })
  if (!isSetupCompleteResponse(response)) throw invalidResponseError()
  return response
}

function isDatabaseCheckResponse(value: unknown): value is DatabaseCheckResponse {
  if (!isRecord(value)) return false
  return (
    value.status === "OK" &&
    (value.databaseKind === "SQLITE" ||
      value.databaseKind === "POSTGRESQL" ||
      value.databaseKind === "MYSQL")
  )
}

function isSetupCompleteResponse(value: unknown): value is SetupCompleteResponse {
  return isRecord(value) && value.status === "READY" && isSessionUser(value.user)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}
