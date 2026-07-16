import { apiRequest } from "../../shared/api/client"
import { login } from "../auth/api"
import type { SessionResponse } from "../auth/session"
import type { SetupValues } from "./model"

export async function checkDatabase(values: SetupValues): Promise<void> {
  await apiRequest("/api/v1/setup/database-check", {
    method: "POST",
    headers: { "x-setup-token": values.token },
    body: JSON.stringify({ databaseUrl: values.databaseUrl }),
  })
}

export async function completeSetup(values: SetupValues): Promise<SessionResponse> {
  await apiRequest("/api/v1/setup/complete", {
    method: "POST",
    headers: { "x-setup-token": values.token },
    body: JSON.stringify({
      databaseUrl: values.databaseUrl,
      username: values.username,
      password: values.password,
      email: values.email || null,
    }),
  })
  return login({ login: values.username, password: values.password })
}
