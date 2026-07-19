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

export function invalidResponseError(): ApiClientError {
  return new ApiClientError(502, {
    code: "INVALID_RESPONSE",
    message: "Invalid server response",
  })
}

export async function apiRequest(
  path: string,
  init: RequestInit = {},
): Promise<unknown> {
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
    return undefined
  }
  try {
    return await response.json()
  } catch {
    throw invalidResponseError()
  }
}

export interface ApiBlobResponse {
  blob: Blob
  filename: string | null
}

export async function apiBlobRequest(
  path: string,
  init: RequestInit = {},
): Promise<ApiBlobResponse> {
  const response = await fetch(path, {
    ...init,
    headers: new Headers(init.headers),
    credentials: "same-origin",
  })
  if (!response.ok) {
    throw new ApiClientError(response.status, await readApiError(response))
  }
  return {
    blob: await response.blob(),
    filename: attachmentFilename(response.headers.get("content-disposition")),
  }
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

function attachmentFilename(contentDisposition: string | null): string | null {
  if (!contentDisposition) return null
  const match = /(?:^|;)\s*filename="([^"]+)"(?:;|$)/iu.exec(contentDisposition)
  return match?.[1] ?? null
}
