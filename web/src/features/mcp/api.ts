import { apiRequest, invalidResponseError } from "../../shared/api/client"

export interface McpToken {
  id: string
  name: string
  prefix: string
  createdAt: string
  expiresAt: string
  lastUsedAt: string | null
}

export interface CreatedMcpToken { token: string; item: McpToken }

function isToken(value: unknown): value is McpToken {
  if (!value || typeof value !== "object") return false
  const token = value as Record<string, unknown>
  return [token.id, token.name, token.prefix].every((field) => typeof field === "string") &&
    [token.createdAt, token.expiresAt].every(isDate) &&
    (token.lastUsedAt === null || isDate(token.lastUsedAt))
}

function isDate(value: unknown): boolean {
  return typeof value === "string" && Number.isFinite(Date.parse(value))
}

export async function listMcpTokens(): Promise<McpToken[]> {
  const response = await apiRequest("/api/v1/mcp/tokens")
  if (!response || typeof response !== "object" || !("items" in response) ||
      !Array.isArray(response.items) || !response.items.every(isToken)) throw invalidResponseError()
  return response.items
}

export async function createMcpToken(name: string, expiresInDays: number, csrfToken: string): Promise<CreatedMcpToken> {
  const response = await apiRequest("/api/v1/mcp/tokens", {
    method: "POST", headers: { "x-csrf-token": csrfToken }, body: JSON.stringify({ name, expiresInDays }),
  })
  if (!response || typeof response !== "object" || !("token" in response) ||
      typeof response.token !== "string" || !("item" in response) || !isToken(response.item)) throw invalidResponseError()
  return { token: response.token, item: response.item }
}

export async function revokeMcpToken(id: string, csrfToken: string): Promise<void> {
  await apiRequest(`/api/v1/mcp/tokens/${encodeURIComponent(id)}`, {
    method: "DELETE", headers: { "x-csrf-token": csrfToken },
  })
}
