export type BackupTargetKind = "S3" | "WEBDAV"

export interface S3Settings {
  endpoint: string
  region: string
  bucket: string
  prefix: string
  pathStyle: boolean
}

export interface WebDavSettings {
  endpoint: string
  prefix: string
}

export type BackupPublicConfig =
  | { kind: "S3"; settings: S3Settings }
  | { kind: "WEBDAV"; settings: WebDavSettings }

export interface RetentionPolicy {
  retainCount: number | null
  retainDays: number | null
}

export interface BackupTarget {
  targetId: string
  displayName: string
  enabled: boolean
  config: BackupPublicConfig
  retention: RetentionPolicy
  revision: number
  hasCredentials: boolean
  createdAt: string
  updatedAt: string
}

export interface BackupSchedule {
  enabled: boolean
  intervalHours: number
  targetIds: string[]
  nextRunAt: string | null
  revision: number
}

export type BackupTriggerKind = "MANUAL" | "SCHEDULED"
export type BackupJobStatus = "QUEUED" | "RUNNING" | "SUCCEEDED" | "PARTIAL" | "FAILED"
export type BackupJobTargetStatus = "QUEUED" | "RUNNING" | "SUCCEEDED" | "FAILED"

export interface BackupJobTarget {
  targetResultId: string
  targetId: string | null
  targetKind: BackupTargetKind
  targetName: string
  status: BackupJobTargetStatus
  byteSize: number | null
  errorCode: string | null
  startedAt: string | null
  completedAt: string | null
}

export interface BackupJob {
  jobId: string
  triggerKind: BackupTriggerKind
  status: BackupJobStatus
  targetCount: number
  lastErrorCode: string | null
  createdAt: string
  startedAt: string | null
  completedAt: string | null
  targets: BackupJobTarget[]
}

export type BackupCredentials =
  | {
      kind: "S3"
      values: {
        accessKeyId: string
        secretAccessKey: string
        sessionToken: string | null
      }
    }
  | { kind: "WEBDAV"; values: { username: string; password: string } }

export interface SaveBackupTargetRequest {
  displayName: string
  enabled: boolean
  config: BackupPublicConfig
  credentials?: BackupCredentials
  retention: RetentionPolicy
}

export interface SaveBackupScheduleRequest {
  enabled: boolean
  intervalHours: number
  targetIds: string[]
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || (typeof value === "number" && Number.isFinite(value))
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string"
}

export function isBackupTarget(value: unknown): value is BackupTarget {
  if (!isRecord(value) || !isRecord(value.config) || !isRecord(value.retention)) return false
  const kind = value.config.kind
  const settings = value.config.settings
  if (!isRecord(settings)) return false
  const validConfig = kind === "S3"
    ? typeof settings.endpoint === "string"
      && typeof settings.region === "string"
      && typeof settings.bucket === "string"
      && typeof settings.prefix === "string"
      && typeof settings.pathStyle === "boolean"
    : kind === "WEBDAV"
      && typeof settings.endpoint === "string"
      && typeof settings.prefix === "string"
  return Boolean(
    validConfig
      && typeof value.targetId === "string"
      && typeof value.displayName === "string"
      && typeof value.enabled === "boolean"
      && isNullableNumber(value.retention.retainCount)
      && isNullableNumber(value.retention.retainDays)
      && typeof value.revision === "number"
      && typeof value.hasCredentials === "boolean"
      && typeof value.createdAt === "string"
      && typeof value.updatedAt === "string",
  )
}

export function isBackupTargetList(value: unknown): value is { items: BackupTarget[] } {
  return isRecord(value) && Array.isArray(value.items) && value.items.every(isBackupTarget)
}

export function isBackupSchedule(value: unknown): value is BackupSchedule {
  return isRecord(value)
    && typeof value.enabled === "boolean"
    && typeof value.intervalHours === "number"
    && Array.isArray(value.targetIds)
    && value.targetIds.every((item) => typeof item === "string")
    && isNullableString(value.nextRunAt)
    && typeof value.revision === "number"
}

function isBackupJobTarget(value: unknown): value is BackupJobTarget {
  return isRecord(value)
    && typeof value.targetResultId === "string"
    && isNullableString(value.targetId)
    && (value.targetKind === "S3" || value.targetKind === "WEBDAV")
    && typeof value.targetName === "string"
    && ["QUEUED", "RUNNING", "SUCCEEDED", "FAILED"].includes(String(value.status))
    && isNullableNumber(value.byteSize)
    && isNullableString(value.errorCode)
    && isNullableString(value.startedAt)
    && isNullableString(value.completedAt)
}

export function isBackupJob(value: unknown): value is BackupJob {
  return isRecord(value)
    && typeof value.jobId === "string"
    && (value.triggerKind === "MANUAL" || value.triggerKind === "SCHEDULED")
    && ["QUEUED", "RUNNING", "SUCCEEDED", "PARTIAL", "FAILED"].includes(String(value.status))
    && typeof value.targetCount === "number"
    && isNullableString(value.lastErrorCode)
    && typeof value.createdAt === "string"
    && isNullableString(value.startedAt)
    && isNullableString(value.completedAt)
    && Array.isArray(value.targets)
    && value.targets.every(isBackupJobTarget)
}

export function isBackupJobList(value: unknown): value is { items: BackupJob[] } {
  return isRecord(value) && Array.isArray(value.items) && value.items.every(isBackupJob)
}

export function isTestResult(value: unknown): value is { ok: true } {
  return isRecord(value) && value.ok === true
}
