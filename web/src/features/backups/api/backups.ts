import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isBackupJob,
  isBackupJobList,
  isBackupSchedule,
  isBackupTarget,
  isBackupTargetList,
  isTestResult,
  type BackupJob,
  type BackupSchedule,
  type BackupTarget,
  type SaveBackupScheduleRequest,
  type SaveBackupTargetRequest,
} from "./backup.generated"

const BASE = "/api/v1/backups"

export async function listBackupTargets(signal?: AbortSignal): Promise<BackupTarget[]> {
  const value = await apiRequest(`${BASE}/targets`, { signal })
  if (!isBackupTargetList(value)) throw invalidResponseError()
  return value.items
}

export async function createBackupTarget(
  csrfToken: string,
  request: SaveBackupTargetRequest & { credentials: NonNullable<SaveBackupTargetRequest["credentials"]> },
  signal?: AbortSignal,
): Promise<BackupTarget> {
  const value = await apiRequest(`${BASE}/targets`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isBackupTarget(value)) throw invalidResponseError()
  return value
}

export async function updateBackupTarget(
  targetId: string,
  csrfToken: string,
  request: SaveBackupTargetRequest,
  signal?: AbortSignal,
): Promise<BackupTarget> {
  const value = await apiRequest(`${BASE}/targets/${encodeURIComponent(targetId)}`, {
    method: "PATCH",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isBackupTarget(value)) throw invalidResponseError()
  return value
}

export async function deleteBackupTarget(
  targetId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<void> {
  await apiRequest(`${BASE}/targets/${encodeURIComponent(targetId)}`, {
    method: "DELETE",
    headers: { "x-csrf-token": csrfToken },
    signal,
  })
}

export async function testBackupTarget(
  targetId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<void> {
  const value = await apiRequest(`${BASE}/targets/${encodeURIComponent(targetId)}/test`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    signal,
  })
  if (!isTestResult(value)) throw invalidResponseError()
}

export async function getBackupSchedule(signal?: AbortSignal): Promise<BackupSchedule> {
  const value = await apiRequest(`${BASE}/schedule`, { signal })
  if (!isBackupSchedule(value)) throw invalidResponseError()
  return value
}

export async function saveBackupSchedule(
  csrfToken: string,
  request: SaveBackupScheduleRequest,
  signal?: AbortSignal,
): Promise<BackupSchedule> {
  const value = await apiRequest(`${BASE}/schedule`, {
    method: "PUT",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isBackupSchedule(value)) throw invalidResponseError()
  return value
}

export async function createBackupJob(
  csrfToken: string,
  targetIds: string[],
  signal?: AbortSignal,
): Promise<BackupJob> {
  const value = await apiRequest(`${BASE}/jobs`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify({ targetIds }),
    signal,
  })
  if (!isBackupJob(value)) throw invalidResponseError()
  return value
}

export async function listBackupJobs(signal?: AbortSignal): Promise<BackupJob[]> {
  const since = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString()
  const value = await apiRequest(`${BASE}/jobs?since=${encodeURIComponent(since)}&limit=100`, {
    signal,
  })
  if (!isBackupJobList(value)) throw invalidResponseError()
  return value.items
}
