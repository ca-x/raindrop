import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import { isTamboSettings, type TamboSettings, type PutTamboSettingsRequest } from "./tambo.generated"

export type { TamboSettings } from "./tambo.generated"

export async function loadTamboSettings(signal?: AbortSignal): Promise<TamboSettings> {
  return parseSettings(await apiRequest("/api/v1/ai/tambo", { signal }))
}

export async function saveTamboSettings(settings: TamboSettings, enabled: boolean, csrfToken: string): Promise<TamboSettings> {
  const request: PutTamboSettingsRequest = { expectedRevision: settings.revision, enabled }
  return parseSettings(await apiRequest("/api/v1/ai/tambo", {
    method: "PUT",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
  }))
}

function parseSettings(value: unknown): TamboSettings {
  if (!isTamboSettings(value)) throw invalidResponseError()
  const settings = value
  if (settings.configured) {
    if (typeof settings.origin !== "string") throw invalidResponseError()
    try {
      const url = new URL(settings.origin)
      if (!["http:", "https:"].includes(url.protocol) || url.origin !== settings.origin) throw invalidResponseError()
    } catch { throw invalidResponseError() }
  } else if (settings.origin !== null) throw invalidResponseError()
  return settings
}
