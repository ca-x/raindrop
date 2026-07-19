import {
  apiBlobRequest,
  apiRequest,
  invalidResponseError,
  type ApiBlobResponse,
} from "../../shared/api/client"
import { isOpmlImportSummary, type OpmlImportSummary } from "./model"

const IMPORT_PATH = "/api/v1/imports/opml"
const EXPORT_PATH = "/api/v1/exports/opml"

export async function previewOpml(
  file: File,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<OpmlImportSummary> {
  return importRequest(file, csrfToken, "preview", signal)
}

export async function commitOpml(
  file: File,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<OpmlImportSummary> {
  return importRequest(file, csrfToken, "commit", signal)
}

export async function exportOpml(signal?: AbortSignal): Promise<ApiBlobResponse> {
  return apiBlobRequest(EXPORT_PATH, { signal })
}

async function importRequest(
  file: File,
  csrfToken: string,
  mode: "preview" | "commit",
  signal?: AbortSignal,
): Promise<OpmlImportSummary> {
  const response = await apiRequest(`${IMPORT_PATH}?mode=${mode}`, {
    method: "POST",
    headers: {
      "content-type": "application/xml",
      "x-csrf-token": csrfToken,
    },
    body: file,
    signal,
  })
  if (!isOpmlImportSummary(response)) throw invalidResponseError()
  return response
}
