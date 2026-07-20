import { useCallback, useEffect, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import {
  enqueueAiJob,
  getAiJob,
  getAiJobResult,
  getEntryAiOverview,
  retryAiJob,
} from "../api/content"
import type {
  AiArtifact,
  AiJob,
  AiOperationOverview,
  EnqueueAiJobRequest,
  EntryAiOverview,
  RetryAiJobRequest,
} from "../api/content.generated"

export interface EntryAiApi {
  getEntryAiOverview: typeof getEntryAiOverview
  enqueueAiJob: typeof enqueueAiJob
  getAiJob: typeof getAiJob
  getAiJobResult: typeof getAiJobResult
  retryAiJob: typeof retryAiJob
}

export type EntryAiTab = "summary" | "translation"
export type EntryAiLoadStatus = "idle" | "loading" | "ready" | "error"
export type EntryAiError = "LOAD" | "MUTATION" | "POLL" | "RESULT"

export interface EntryAiController {
  entryId: string | null
  openTab: EntryAiTab | null
  overview: EntryAiOverview | null
  loadStatus: EntryAiLoadStatus
  error: EntryAiError | null
  isMutating: boolean
  open: (tab: EntryAiTab) => void
  close: () => void
  enqueue: (tab: EntryAiTab) => Promise<boolean>
  retry: (tab: EntryAiTab) => Promise<boolean>
  clearError: () => void
}

interface EntryAiControllerOptions {
  api?: EntryAiApi
  pollIntervalMs?: number
  idempotencyKey?: () => string
}

const defaultApi: EntryAiApi = {
  getEntryAiOverview,
  enqueueAiJob,
  getAiJob,
  getAiJobResult,
  retryAiJob,
}

export function useEntryAiController(
  entryId: string | null,
  csrfToken: string,
  onUnauthenticated: () => void,
  options: EntryAiControllerOptions = {},
): EntryAiController {
  const api = options.api ?? defaultApi
  const pollIntervalMs = options.pollIntervalMs ?? 1000
  const idempotencyKey = useRef(options.idempotencyKey ?? defaultIdempotencyKey)
  const onUnauthenticatedRef = useRef(onUnauthenticated)
  const [openTab, setOpenTab] = useState<EntryAiTab | null>(null)
  const [overview, setOverview] = useState<EntryAiOverview | null>(null)
  const [loadStatus, setLoadStatus] = useState<EntryAiLoadStatus>("idle")
  const [error, setError] = useState<EntryAiError | null>(null)
  const [isMutating, setIsMutating] = useState(false)
  const entryIdRef = useRef(entryId)
  const openTabRef = useRef<EntryAiTab | null>(null)
  const loadedEntryId = useRef<string | null>(null)
  const overviewGeneration = useRef(0)
  const overviewAbort = useRef<AbortController | null>(null)
  const mutationAbort = useRef<AbortController | null>(null)
  const pollAbort = useRef<AbortController | null>(null)
  const mutating = useRef(false)
  entryIdRef.current = entryId
  onUnauthenticatedRef.current = onUnauthenticated

  const endSession = useCallback(() => {
    overviewGeneration.current += 1
    overviewAbort.current?.abort()
    mutationAbort.current?.abort()
    pollAbort.current?.abort()
    onUnauthenticatedRef.current()
  }, [])

  const loadOverview = useCallback(
    async (requestedEntryId: string) => {
      overviewAbort.current?.abort()
      const generation = ++overviewGeneration.current
      const abort = new AbortController()
      overviewAbort.current = abort
      setLoadStatus("loading")
      setError(null)
      try {
        const loaded = await api.getEntryAiOverview(
          requestedEntryId,
          undefined,
          abort.signal,
        )
        if (
          generation !== overviewGeneration.current ||
          entryIdRef.current !== requestedEntryId
        ) {
          return
        }
        loadedEntryId.current = requestedEntryId
        setOverview(loaded)
        setLoadStatus("ready")
      } catch (cause) {
        if (
          generation !== overviewGeneration.current ||
          isAbortError(cause)
        ) {
          return
        }
        if (isAuthenticationError(cause)) {
          endSession()
          return
        }
        setLoadStatus("error")
        setError("LOAD")
      } finally {
        if (generation === overviewGeneration.current) overviewAbort.current = null
      }
    },
    [api, endSession],
  )

  useEffect(() => {
    overviewGeneration.current += 1
    overviewAbort.current?.abort()
    mutationAbort.current?.abort()
    pollAbort.current?.abort()
    overviewAbort.current = null
    mutationAbort.current = null
    pollAbort.current = null
    loadedEntryId.current = null
    setOverview(null)
    setLoadStatus("idle")
    setError(null)
    setIsMutating(false)
    mutating.current = false
    if (entryId && openTabRef.current) void loadOverview(entryId)
  }, [entryId, loadOverview])

  useEffect(
    () => () => {
      overviewGeneration.current += 1
      overviewAbort.current?.abort()
      mutationAbort.current?.abort()
      pollAbort.current?.abort()
    },
    [],
  )

  const open = useCallback(
    (tab: EntryAiTab) => {
      openTabRef.current = tab
      setOpenTab(tab)
      setError(null)
      if (entryId && loadedEntryId.current !== entryId) {
        void loadOverview(entryId)
      }
    },
    [entryId, loadOverview],
  )

  const close = useCallback(() => {
    openTabRef.current = null
    pollAbort.current?.abort()
    pollAbort.current = null
    setOpenTab(null)
    setError(null)
  }, [])

  const enqueue = useCallback(
    async (tab: EntryAiTab) => {
      if (!entryId || !overview || mutating.current) return false
      const operation = operationForTab(overview, tab)
      const request: EnqueueAiJobRequest = {
        operation: tab === "summary" ? "SUMMARIZE" : "TRANSLATE",
        targetLocale: tab === "summary" ? null : operation.targetLocale ?? null,
        idempotencyKey: idempotencyKey.current(),
      }
      const abort = new AbortController()
      mutationAbort.current = abort
      mutating.current = true
      setIsMutating(true)
      setError(null)
      try {
        const job = await api.enqueueAiJob(entryId, csrfToken, request, abort.signal)
        if (entryIdRef.current !== entryId) return false
        setOverview((current) =>
          current ? updateOperationJob(current, tab, job) : current,
        )
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        setError("MUTATION")
        return false
      } finally {
        if (mutationAbort.current === abort) {
          mutationAbort.current = null
          mutating.current = false
          setIsMutating(false)
        }
      }
    },
    [api, csrfToken, endSession, entryId, overview],
  )

  const retry = useCallback(
    async (tab: EntryAiTab) => {
      if (!overview || mutating.current) return false
      const requestedEntryId = entryIdRef.current
      if (!requestedEntryId) return false
      const failedJob = operationForTab(overview, tab).job
      if (!failedJob || failedJob.status !== "FAILED") return false
      const request: RetryAiJobRequest = {
        idempotencyKey: idempotencyKey.current(),
      }
      const abort = new AbortController()
      mutationAbort.current = abort
      mutating.current = true
      setIsMutating(true)
      setError(null)
      try {
        const job = await api.retryAiJob(
          failedJob.jobId,
          csrfToken,
          request,
          abort.signal,
        )
        if (entryIdRef.current !== requestedEntryId) return false
        setOverview((current) =>
          current ? updateOperationJob(current, tab, job) : current,
        )
        return true
      } catch (cause) {
        if (isAbortError(cause)) return false
        if (isAuthenticationError(cause)) {
          endSession()
          return false
        }
        setError("MUTATION")
        return false
      } finally {
        if (mutationAbort.current === abort) {
          mutationAbort.current = null
          mutating.current = false
          setIsMutating(false)
        }
      }
    },
    [api, csrfToken, endSession, overview],
  )

  const activeOperation =
    openTab && overview ? operationForTab(overview, openTab) : null
  const activeJob = activeOperation?.job ?? null
  const activeArtifact = activeOperation?.artifact ?? null
  useEffect(() => {
    if (!entryId || !openTab || !activeJob) return
    const needsResult = activeJob.status === "SUCCEEDED" && activeArtifact === null
    if (!needsResult && !isActiveJob(activeJob)) return

    const abort = new AbortController()
    pollAbort.current?.abort()
    pollAbort.current = abort
    let timer: number | null = null
    let cancelled = false

    const fail = (cause: unknown, kind: EntryAiError) => {
      if (cancelled || isAbortError(cause)) return
      if (isAuthenticationError(cause)) {
        endSession()
        return
      }
      setError(kind)
    }
    const loadResult = async (job: AiJob) => {
      try {
        const artifact = await api.getAiJobResult(job.jobId, abort.signal)
        if (cancelled || entryIdRef.current !== entryId) return
        if (!artifactMatchesTab(artifact, openTab)) {
          setError("RESULT")
          return
        }
        setOverview((current) =>
          current ? updateOperationArtifact(current, openTab, job, artifact) : current,
        )
      } catch (cause) {
        fail(cause, "RESULT")
      }
    }
    const poll = async () => {
      timer = window.setTimeout(async () => {
        timer = null
        try {
          const job = await api.getAiJob(activeJob.jobId, abort.signal)
          if (cancelled || entryIdRef.current !== entryId) return
          setOverview((current) =>
            current ? updateOperationJob(current, openTab, job) : current,
          )
          if (job.status === "SUCCEEDED") {
            await loadResult(job)
          } else if (isActiveJob(job)) {
            await poll()
          }
        } catch (cause) {
          fail(cause, "POLL")
        }
      }, pollIntervalMs)
    }

    if (needsResult) void loadResult(activeJob)
    else void poll()
    return () => {
      cancelled = true
      abort.abort()
      if (timer !== null) window.clearTimeout(timer)
      if (pollAbort.current === abort) pollAbort.current = null
    }
  }, [
    activeArtifact,
    activeJob,
    api,
    endSession,
    entryId,
    openTab,
    pollIntervalMs,
  ])

  return {
    entryId,
    openTab,
    overview,
    loadStatus,
    error,
    isMutating,
    open,
    close,
    enqueue,
    retry,
    clearError: useCallback(() => setError(null), []),
  }
}

function operationForTab(
  overview: EntryAiOverview,
  tab: EntryAiTab,
): AiOperationOverview {
  return tab === "summary" ? overview.summary : overview.translation
}

function updateOperationJob(
  overview: EntryAiOverview,
  tab: EntryAiTab,
  job: AiJob,
): EntryAiOverview {
  const current = operationForTab(overview, tab)
  const updated: AiOperationOverview = {
    ...current,
    state: jobState(job),
    job,
    artifact: job.status === "SUCCEEDED" ? current.artifact : null,
  }
  return tab === "summary"
    ? { ...overview, summary: updated }
    : { ...overview, translation: updated }
}

function updateOperationArtifact(
  overview: EntryAiOverview,
  tab: EntryAiTab,
  job: AiJob,
  artifact: AiArtifact,
): EntryAiOverview {
  const updated: AiOperationOverview = {
    ...operationForTab(overview, tab),
    state: "SUCCEEDED",
    job,
    artifact,
  }
  return tab === "summary"
    ? { ...overview, summary: updated }
    : { ...overview, translation: updated }
}

function jobState(job: AiJob): AiOperationOverview["state"] {
  return job.status
}

function isActiveJob(job: AiJob): boolean {
  return (
    job.status === "QUEUED" ||
    job.status === "RUNNING" ||
    job.status === "RETRY_WAIT"
  )
}

function artifactMatchesTab(artifact: AiArtifact, tab: EntryAiTab): boolean {
  return tab === "summary"
    ? artifact.kind === "AI_SUMMARY"
    : artifact.kind === "AI_TRANSLATION"
}

function defaultIdempotencyKey(): string {
  return `reader:${globalThis.crypto.randomUUID()}`
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof ApiClientError && error.status === 401
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError"
}
