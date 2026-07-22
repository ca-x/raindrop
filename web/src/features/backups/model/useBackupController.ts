import { useCallback, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  createBackupJob,
  createBackupTarget,
  deleteBackupTarget,
  getBackupSchedule,
  listBackupJobs,
  listBackupTargets,
  saveBackupSchedule,
  testBackupTarget,
  updateBackupTarget,
} from "../api/backups"
import type {
  BackupCredentials,
  BackupJob,
  BackupSchedule,
  BackupTarget,
  SaveBackupScheduleRequest,
  SaveBackupTargetRequest,
} from "../api/backup.generated"

export type BackupControllerError =
  | "LOAD"
  | "VALIDATION"
  | "SAVE"
  | "CONFLICT"
  | "SECRET_UNAVAILABLE"
  | "TEST"
  | "TARGET_UNREACHABLE"
  | "RATE_LIMITED"

export interface BackupController {
  targets: BackupTarget[]
  schedule: BackupSchedule
  jobs: BackupJob[]
  loadStatus: "idle" | "loading" | "ready" | "error"
  error: BackupControllerError | null
  isMutating: boolean
  activeTargetId: string | null
  load: () => Promise<void>
  refreshJobs: () => Promise<void>
  createTarget: (
    request: SaveBackupTargetRequest & { credentials: BackupCredentials },
  ) => Promise<boolean>
  updateTarget: (targetId: string, request: SaveBackupTargetRequest) => Promise<boolean>
  deleteTarget: (targetId: string) => Promise<boolean>
  testTarget: (targetId: string) => Promise<boolean>
  saveSchedule: (request: SaveBackupScheduleRequest) => Promise<boolean>
  runNow: (targetIds: string[]) => Promise<boolean>
  clearError: () => void
  cancel: () => void
}

interface Options {
  csrfToken: string
  onUnauthenticated: () => void
}

const defaultSchedule: BackupSchedule = {
  enabled: false,
  intervalHours: 24,
  targetIds: [],
  nextRunAt: null,
  revision: 0,
}

export function useBackupController({
  csrfToken,
  onUnauthenticated,
}: Options): BackupController {
  const [targets, setTargets] = useState<BackupTarget[]>([])
  const [schedule, setSchedule] = useState(defaultSchedule)
  const [jobs, setJobs] = useState<BackupJob[]>([])
  const [loadStatus, setLoadStatus] = useState<BackupController["loadStatus"]>("idle")
  const [error, setError] = useState<BackupControllerError | null>(null)
  const [isMutating, setIsMutating] = useState(false)
  const [activeTargetId, setActiveTargetId] = useState<string | null>(null)
  const generation = useRef(0)
  const abortRef = useRef<AbortController | null>(null)

  const endSession = useCallback(() => {
    generation.current += 1
    abortRef.current?.abort()
    onUnauthenticated()
  }, [onUnauthenticated])

  const load = useCallback(async () => {
    const current = ++generation.current
    abortRef.current?.abort()
    const abort = new AbortController()
    abortRef.current = abort
    setLoadStatus("loading")
    setError(null)
    try {
      const [nextTargets, nextSchedule, nextJobs] = await Promise.all([
        listBackupTargets(abort.signal),
        getBackupSchedule(abort.signal),
        listBackupJobs(abort.signal),
      ])
      if (current !== generation.current) return
      setTargets(nextTargets)
      setSchedule(nextSchedule)
      setJobs(nextJobs)
      setLoadStatus("ready")
    } catch (cause) {
      if (current !== generation.current || isAbort(cause)) return
      if (isUnauthenticated(cause)) return endSession()
      setError("LOAD")
      setLoadStatus("error")
    } finally {
      if (abortRef.current === abort) abortRef.current = null
    }
  }, [endSession])

  const refreshJobs = useCallback(async () => {
    try {
      setJobs(await listBackupJobs())
    } catch (cause) {
      if (isUnauthenticated(cause)) return endSession()
      setError("LOAD")
    }
  }, [endSession])

  const mutate = useCallback(async <T,>(
    targetId: string | null,
    operation: () => Promise<T>,
    onSuccess: (value: T) => void,
    fallback: BackupControllerError,
  ) => {
    if (isMutating) return false
    setIsMutating(true)
    setActiveTargetId(targetId)
    setError(null)
    try {
      const value = await operation()
      onSuccess(value)
      return true
    } catch (cause) {
      if (isUnauthenticated(cause)) {
        endSession()
        return false
      }
      setError(mapError(cause, fallback))
      return false
    } finally {
      setIsMutating(false)
      setActiveTargetId(null)
    }
  }, [csrfToken, endSession, isMutating])

  return {
    targets,
    schedule,
    jobs,
    loadStatus,
    error,
    isMutating,
    activeTargetId,
    load,
    refreshJobs,
    createTarget: useCallback((request) => mutate(
      null,
      () => createBackupTarget(csrfToken, request),
      (target) => setTargets((current) => [...current, target].sort(compareTargets)),
      "SAVE",
    ), [csrfToken, mutate]),
    updateTarget: useCallback((targetId, request) => mutate(
      targetId,
      () => updateBackupTarget(targetId, csrfToken, request),
      (target) => setTargets((current) => current
        .map((candidate) => candidate.targetId === targetId ? target : candidate)
        .sort(compareTargets)),
      "SAVE",
    ), [csrfToken, mutate]),
    deleteTarget: useCallback((targetId) => mutate(
      targetId,
      () => deleteBackupTarget(targetId, csrfToken),
      () => {
        setTargets((current) => current.filter((target) => target.targetId !== targetId))
        setSchedule((current) => ({
          ...current,
          targetIds: current.targetIds.filter((id) => id !== targetId),
        }))
      },
      "SAVE",
    ), [csrfToken, mutate]),
    testTarget: useCallback((targetId) => mutate(
      targetId,
      () => testBackupTarget(targetId, csrfToken),
      () => undefined,
      "TEST",
    ), [csrfToken, mutate]),
    saveSchedule: useCallback((request) => mutate(
      null,
      () => saveBackupSchedule(csrfToken, request),
      setSchedule,
      "SAVE",
    ), [csrfToken, mutate]),
    runNow: useCallback((targetIds) => mutate(
      null,
      () => createBackupJob(csrfToken, targetIds),
      (job) => setJobs((current) => [job, ...current]),
      "SAVE",
    ), [csrfToken, mutate]),
    clearError: useCallback(() => setError(null), []),
    cancel: useCallback(() => {
      generation.current += 1
      abortRef.current?.abort()
      abortRef.current = null
    }, []),
  }
}

function compareTargets(left: BackupTarget, right: BackupTarget): number {
  return left.config.kind.localeCompare(right.config.kind)
    || left.displayName.localeCompare(right.displayName)
}

function mapError(cause: unknown, fallback: BackupControllerError): BackupControllerError {
  if (!(cause instanceof ApiClientError)) return fallback
  if (cause.status === 422) return "VALIDATION"
  if (cause.status === 429) return "RATE_LIMITED"
  if (cause.payload.code === "CONFLICT") return "CONFLICT"
  if (cause.payload.code === "BACKUP_KEYRING_UNAVAILABLE") return "SECRET_UNAVAILABLE"
  if (cause.payload.code === "TARGET_UNREACHABLE" || cause.payload.code === "TARGET_AUTH_FAILED") {
    return "TARGET_UNREACHABLE"
  }
  return fallback
}

function isUnauthenticated(cause: unknown): boolean {
  return cause instanceof ApiClientError && cause.status === 401
}

function isAbort(cause: unknown): boolean {
  return cause instanceof DOMException && cause.name === "AbortError"
}
