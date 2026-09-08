import { apiRequest, invalidResponseError } from "../../shared/api/client"

export interface DatabaseStorage {
  databaseBytes: number
  walBytes: number
  freeBytes: number
  entryCount: number
  feedCount: number
  tables: { name: string; bytes: number }[] | null
}
export interface ArticleRetentionSettings { enabled: boolean; retentionDays: number }
export interface MaintenanceStatus {
  articleRetention: ArticleRetentionSettings
  running: boolean
  storage: DatabaseStorage | null
  result: {
    before: DatabaseStorage
    after: DatabaseStorage
    reclaimedBytes: number
    walTruncated: boolean
    removedRefreshRuns: number
  } | null
  error: string | null
}
function isStorage(value: unknown): value is DatabaseStorage {
  if (!value || typeof value !== "object") return false
  const v = value as Record<string, unknown>
  return [v.databaseBytes, v.walBytes, v.freeBytes, v.entryCount, v.feedCount].every(isCount) &&
    (v.tables === null || (Array.isArray(v.tables) && v.tables.every((table: unknown) => {
      if (!table || typeof table !== "object") return false
      const t = table as Record<string, unknown>
      return typeof t.name === "string" && isCount(t.bytes)
    })))
}
function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
}
export async function databaseRequest(csrfToken?: string): Promise<MaintenanceStatus> {
  const value = await apiRequest(csrfToken === undefined ? "/api/v1/database" : "/api/v1/database/compact",
    csrfToken === undefined ? {} : { method: "POST", headers: { "x-csrf-token": csrfToken } })
  if (!value || typeof value !== "object") throw invalidResponseError()
  const v = value as MaintenanceStatus
  if (!isArticleRetention(v.articleRetention) || typeof v.running !== "boolean" || !(v.storage === null || isStorage(v.storage)) ||
      !(v.error === null || typeof v.error === "string") ||
      !(v.result === null || (typeof v.result === "object" && isStorage(v.result.before) &&
        isStorage(v.result.after) && isCount(v.result.reclaimedBytes) &&
        isCount(v.result.removedRefreshRuns) && typeof v.result.walTruncated === "boolean"))) {
    throw invalidResponseError()
  }
  return v
}

function isArticleRetention(value: unknown): value is ArticleRetentionSettings {
  if (!value || typeof value !== "object") return false
  const v = value as ArticleRetentionSettings
  return typeof v.enabled === "boolean" && isCount(v.retentionDays) && v.retentionDays >= 1 && v.retentionDays <= 3650
}
export async function saveArticleRetention(settings: ArticleRetentionSettings, csrfToken: string): Promise<ArticleRetentionSettings> {
  const value = await apiRequest("/api/v1/database/article-retention", {
    method: "PUT", headers: { "x-csrf-token": csrfToken }, body: JSON.stringify(settings),
  })
  if (!isArticleRetention(value)) throw invalidResponseError()
  return value
}
