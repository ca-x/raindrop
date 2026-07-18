import { ApiClientError } from "../../../shared/api/client"

export const GENERIC_READER_ERROR = "Something went wrong. Please try again."

const STABLE_MESSAGE_STATUSES = new Set([403, 409, 422, 429])

export function isUnauthenticatedError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

export function readerErrorMessage(error: unknown): string {
  if (error instanceof ApiClientError && STABLE_MESSAGE_STATUSES.has(error.status)) {
    return error.payload.message
  }
  return GENERIC_READER_ERROR
}

export function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
