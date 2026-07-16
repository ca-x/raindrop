export interface ApiErrorPayload {
  code: string
  message: string
  fields?: Record<string, string>
  requestId?: string
}

export class ApiClientError extends Error {
  constructor(
    readonly status: number,
    readonly payload: ApiErrorPayload,
  ) {
    super(payload.message)
    this.name = "ApiClientError"
  }
}

export async function apiRequest<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json")
  }
  const response = await fetch(path, {
    ...init,
    headers,
    credentials: "same-origin",
  })
  if (!response.ok) {
    throw new ApiClientError(response.status, await readApiError(response))
  }
  if (response.status === 204) {
    return undefined as T
  }
  return (await response.json()) as T
}

async function readApiError(response: Response): Promise<ApiErrorPayload> {
  try {
    const body: unknown = await response.json()
    if (isRecord(body) && isRecord(body.error) && typeof body.error.code === "string") {
      return {
        code: body.error.code,
        message:
          typeof body.error.message === "string" ? body.error.message : "Request failed",
        fields: isStringRecord(body.error.fields) ? body.error.fields : undefined,
        requestId:
          typeof body.error.requestId === "string" ? body.error.requestId : undefined,
      }
    }
  } catch {
    // The stable fallback keeps server internals out of the rendered UI.
  }
  return { code: "REQUEST_FAILED", message: "Request failed" }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return (
    isRecord(value) && Object.values(value).every((entry) => typeof entry === "string")
  )
}
