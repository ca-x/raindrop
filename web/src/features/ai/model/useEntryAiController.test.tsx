import { act, renderHook, waitFor } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import { ApiClientError } from "../../../shared/api/client"
import type {
  AiArtifact,
  AiJob,
  EnqueueAiJobRequest,
  EntryAiOverview,
  RetryAiJobRequest,
} from "../api/content.generated"
import {
  useEntryAiController,
  type EntryAiApi,
} from "./useEntryAiController"

const firstEntryId = "00000000-0000-4000-8000-000000000301"
const secondEntryId = "00000000-0000-4000-8000-000000000302"
const jobId = "00000000-0000-4000-8000-000000000401"
const queuedJob: AiJob = {
  jobId,
  status: "QUEUED",
  attempts: 0,
  maxAttempts: 3,
  nextAttemptAt: "2026-07-20T10:00:00Z",
  lastErrorCode: null,
  createdAt: "2026-07-20T10:00:00Z",
  startedAt: null,
  completedAt: null,
}
const summaryArtifact: AiArtifact = {
  artifactId: "00000000-0000-4000-8000-000000000501",
  kind: "AI_SUMMARY",
  providerLabel: "Primary model",
  createdAt: "2026-07-20T10:00:03Z",
  sourceLanguage: "en",
  summary: "A stable summary.",
  bullets: ["One", "Two"],
  conclusion: null,
}

it("loads lazily once for the selected entry and reuses the overview on reopen", async () => {
  const api = fakeApi()
  const { result } = renderController(firstEntryId, api)

  expect(api.getEntryAiOverview).not.toHaveBeenCalled()
  act(() => result.current.open("summary"))
  await waitFor(() => expect(result.current.loadStatus).toBe("ready"))
  expect(api.getEntryAiOverview).toHaveBeenCalledTimes(1)

  act(() => result.current.close())
  act(() => result.current.open("translation"))
  expect(api.getEntryAiOverview).toHaveBeenCalledTimes(1)
  expect(result.current.openTab).toBe("translation")
})

it("aborts entry changes and suppresses the late overview response", async () => {
  const first = deferred<EntryAiOverview>()
  const second = deferred<EntryAiOverview>()
  const getEntryAiOverview = vi.fn(
    (entryId: string, _locale?: string, _signal?: AbortSignal) =>
      entryId === firstEntryId ? first.promise : second.promise,
  )
  const api = fakeApi({ getEntryAiOverview })
  const { result, rerender } = renderHook(
    ({ entryId }) =>
      useEntryAiController(entryId, "csrf-memory", vi.fn(), {
        api,
        pollIntervalMs: 5,
        idempotencyKey: sequenceKeys(),
      }),
    { initialProps: { entryId: firstEntryId } },
  )

  act(() => result.current.open("summary"))
  const firstSignal = getEntryAiOverview.mock.calls[0]?.[2]
  rerender({ entryId: secondEntryId })
  await waitFor(() => expect(getEntryAiOverview).toHaveBeenCalledTimes(2))
  expect(firstSignal?.aborted).toBe(true)

  first.resolve({ ...idleOverview(), availability: "DISABLED" })
  second.resolve(idleOverview())
  await waitFor(() => expect(result.current.loadStatus).toBe("ready"))
  expect(result.current.entryId).toBe(secondEntryId)
  expect(result.current.overview?.availability).toBe("READY")
})

it("polls active jobs, fetches the typed result, and stops after success", async () => {
  const running = {
    ...queuedJob,
    status: "RUNNING" as const,
    attempts: 1,
    startedAt: "2026-07-20T10:00:01Z",
  }
  const succeeded = {
    ...running,
    status: "SUCCEEDED" as const,
    completedAt: "2026-07-20T10:00:03Z",
  }
  const getAiJob = vi
    .fn()
    .mockResolvedValueOnce(running)
    .mockResolvedValueOnce(succeeded)
  const api = fakeApi({
    getEntryAiOverview: vi.fn().mockResolvedValue({
      ...idleOverview(),
      summary: {
        operation: "SUMMARIZE",
        state: "QUEUED",
        job: queuedJob,
        artifact: null,
      },
    }),
    getAiJob,
    getAiJobResult: vi.fn().mockResolvedValue(summaryArtifact),
  })
  const { result } = renderController(firstEntryId, api)

  act(() => result.current.open("summary"))
  await waitFor(() =>
    expect(result.current.overview?.summary.artifact).toEqual(summaryArtifact),
  )
  expect(getAiJob).toHaveBeenCalledTimes(2)
  expect(api.getAiJobResult).toHaveBeenCalledWith(jobId, expect.any(AbortSignal))

  const callsAtSuccess = getAiJob.mock.calls.length
  await new Promise((resolve) => setTimeout(resolve, 20))
  expect(getAiJob).toHaveBeenCalledTimes(callsAtSuccess)
})

it("does not poll idle or failed operations and stops polling on close", async () => {
  const failedJob = {
    ...queuedJob,
    status: "FAILED" as const,
    attempts: 3,
    lastErrorCode: "PROVIDER_TIMEOUT",
    completedAt: "2026-07-20T10:00:03Z",
  }
  const api = fakeApi({
    getEntryAiOverview: vi.fn().mockResolvedValue({
      ...idleOverview(),
      summary: {
        operation: "SUMMARIZE",
        state: "FAILED",
        job: failedJob,
        artifact: null,
      },
    }),
  })
  const { result } = renderController(firstEntryId, api)

  act(() => result.current.open("summary"))
  await waitFor(() => expect(result.current.loadStatus).toBe("ready"))
  await new Promise((resolve) => setTimeout(resolve, 20))
  expect(api.getAiJob).not.toHaveBeenCalled()

  act(() => result.current.close())
  expect(result.current.openTab).toBeNull()
})

it("enqueues and retries with a fresh idempotency key", async () => {
  const failedJob = {
    ...queuedJob,
    status: "FAILED" as const,
    attempts: 3,
    lastErrorCode: "PROVIDER_TIMEOUT",
    completedAt: "2026-07-20T10:00:03Z",
  }
  const retryJob = {
    ...queuedJob,
    jobId: "00000000-0000-4000-8000-000000000402",
  }
  const api = fakeApi({
    getEntryAiOverview: vi.fn().mockResolvedValue({
      ...idleOverview(),
      summary: {
        operation: "SUMMARIZE",
        state: "FAILED",
        job: failedJob,
        artifact: null,
      },
    }),
    enqueueAiJob: vi.fn().mockResolvedValue(queuedJob),
    retryAiJob: vi.fn().mockResolvedValue(retryJob),
  })
  const { result } = renderController(firstEntryId, api)
  act(() => result.current.open("summary"))
  await waitFor(() => expect(result.current.loadStatus).toBe("ready"))

  await act(async () => result.current.retry("summary"))
  expect(api.retryAiJob).toHaveBeenCalledWith(
    jobId,
    "csrf-memory",
    { idempotencyKey: "key-1" },
    expect.any(AbortSignal),
  )
  expect(result.current.overview?.summary.job?.jobId).toBe(retryJob.jobId)

  await act(async () => result.current.enqueue("translation"))
  expect(api.enqueueAiJob).toHaveBeenCalledWith(
    firstEntryId,
    "csrf-memory",
    {
      operation: "TRANSLATE",
      targetLocale: "zh-CN",
      idempotencyKey: "key-2",
    },
    expect.any(AbortSignal),
  )
})

it("ignores a late retry response after the selected entry changes", async () => {
  const failedJob = {
    ...queuedJob,
    status: "FAILED" as const,
    attempts: 3,
    lastErrorCode: "PROVIDER_TIMEOUT",
    completedAt: "2026-07-20T10:00:03Z",
  }
  const retry = deferred<AiJob>()
  const api = fakeApi({
    getEntryAiOverview: vi.fn().mockImplementation((entryId: string) =>
      Promise.resolve(
        entryId === firstEntryId
          ? {
              ...idleOverview(),
              summary: {
                operation: "SUMMARIZE" as const,
                state: "FAILED" as const,
                job: failedJob,
                artifact: null,
              },
            }
          : idleOverview(),
      ),
    ),
    retryAiJob: vi.fn(() => retry.promise),
  })
  const { result, rerender } = renderHook(
    ({ entryId }) =>
      useEntryAiController(entryId, "csrf-memory", vi.fn(), {
        api,
        pollIntervalMs: 5,
        idempotencyKey: sequenceKeys(),
      }),
    { initialProps: { entryId: firstEntryId } },
  )

  act(() => result.current.open("summary"))
  await waitFor(() => expect(result.current.loadStatus).toBe("ready"))

  let retryRequest!: Promise<boolean>
  act(() => {
    retryRequest = result.current.retry("summary")
  })
  const signal = vi.mocked(api.retryAiJob).mock.calls[0]?.[3]
  rerender({ entryId: secondEntryId })
  await waitFor(() => expect(result.current.entryId).toBe(secondEntryId))
  await waitFor(() => expect(result.current.loadStatus).toBe("ready"))
  expect(signal?.aborted).toBe(true)

  retry.resolve({
    ...queuedJob,
    jobId: "00000000-0000-4000-8000-000000000402",
  })
  await act(async () => expect(retryRequest).resolves.toBe(false))

  expect(result.current.overview?.summary.state).toBe("IDLE")
  expect(result.current.overview?.summary.job).toBeNull()
})

it("propagates unauthenticated overview failures", async () => {
  const onUnauthenticated = vi.fn()
  const api = fakeApi({
    getEntryAiOverview: vi.fn().mockRejectedValue(
      new ApiClientError(401, {
        code: "AUTHENTICATION_REQUIRED",
        message: "Authentication is required",
      }),
    ),
  })
  const { result } = renderController(firstEntryId, api, onUnauthenticated)

  act(() => result.current.open("summary"))
  await waitFor(() => expect(onUnauthenticated).toHaveBeenCalledOnce())
  expect(result.current.error).toBeNull()
})

function renderController(
  entryId: string,
  api: EntryAiApi,
  onUnauthenticated = vi.fn(),
) {
  return renderHook(() =>
    useEntryAiController(entryId, "csrf-memory", onUnauthenticated, {
      api,
      pollIntervalMs: 5,
      idempotencyKey: sequenceKeys(),
    }),
  )
}

function fakeApi(overrides: Partial<EntryAiApi> = {}): EntryAiApi {
  return {
    getEntryAiOverview: vi.fn().mockResolvedValue(idleOverview()),
    enqueueAiJob: vi.fn<
      (
        entryId: string,
        csrfToken: string,
        request: EnqueueAiJobRequest,
        signal?: AbortSignal,
      ) => Promise<AiJob>
    >(),
    getAiJob: vi.fn(),
    getAiJobResult: vi.fn(),
    retryAiJob: vi.fn<
      (
        jobId: string,
        csrfToken: string,
        request: RetryAiJobRequest,
        signal?: AbortSignal,
      ) => Promise<AiJob>
    >(),
    ...overrides,
  }
}

function idleOverview(): EntryAiOverview {
  return {
    availability: "READY",
    mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
    summary: {
      operation: "SUMMARIZE",
      state: "IDLE",
      job: null,
      artifact: null,
    },
    translation: {
      operation: "TRANSLATE",
      targetLocale: "zh-CN",
      state: "IDLE",
      job: null,
      artifact: null,
    },
  }
}

function sequenceKeys() {
  let value = 0
  return () => `key-${++value}`
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}
