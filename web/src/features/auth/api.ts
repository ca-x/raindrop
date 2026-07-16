import { apiRequest, invalidResponseError } from "../../shared/api/client"
import { isSessionResponse, type SessionResponse } from "./session"

export interface LoginInput {
  login: string
  password: string
}

export async function login(input: LoginInput): Promise<SessionResponse> {
  const response: unknown = await apiRequest("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify(input),
  })
  if (!isSessionResponse(response)) {
    throw invalidResponseError()
  }
  return response
}

export async function logout(csrfToken: string): Promise<void> {
  await apiRequest("/api/v1/auth/logout", {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
  })
}
