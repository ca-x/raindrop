import { afterEach, expect, it, vi } from "vitest"

import type {
  AiArtifact,
  AiConfigEnvelope,
  AiJob,
  EnqueueAiJobRequest,
  EntryAiOverview,
  PutAiConfigRequest,
} from "./content.generated"
import {
  enqueueAiJob,
  getAiConfig,
  getAiJob,
  getAiJobResult,
  getEntryAiOverview,
  putAiConfig,
  retryAiJob,
} from "./content"

afterEach(() => vi.unstubAllGlobals())

const entryId = "00000000-0000-4000-8000-000000000301"
const jobId = "00000000-0000-4000-8000-000000000401"
const providerId = "00000000-0000-4000-8000-000000000101"
const job: AiJob = {
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
const artifact: AiArtifact = {
  artifactId: "00000000-0000-4000-8000-000000000501",
  kind: "AI_SUMMARY",
  providerLabel: "Primary model",
  createdAt: "2026-07-20T10:00:00Z",
  sourceLanguage: "en",
  summary: "A short summary.",
  bullets: ["One", "Two"],
  conclusion: null,
}
const configEnvelope: AiConfigEnvelope = {
  pluginState: "READY",
  mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
  config: {
    revision: 2,
    isEnabled: true,
    summary: {
      enabled: true,
      providerId,
      style: "BALANCED",
      maxOutputTokens: 1024,
    },
    translation: {
      enabled: false,
      providerId,
      defaultTargetLocale: "zh-CN",
      maxOutputTokens: 4096,
    },
  },
}
const overview: EntryAiOverview = {
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

it("gets and replaces AI config with generated validation and CSRF", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(configEnvelope))
    .mockResolvedValueOnce(jsonResponse(configEnvelope))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal
  const request: PutAiConfigRequest = {
    expectedRevision: 2,
    isEnabled: true,
    summary: configEnvelope.config!.summary,
    translation: configEnvelope.config!.translation,
  }

  await expect(getAiConfig(signal)).resolves.toEqual(configEnvelope)
  await expect(putAiConfig("csrf-memory", request, signal)).resolves.toEqual(
    configEnvelope,
  )

  expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/v1/ai/config")
  const [, init] = fetchMock.mock.calls[1] ?? []
  expect(init?.method).toBe("PUT")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual(request)
})

it("loads entry overview with an encoded optional translation locale", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(overview))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await expect(getEntryAiOverview(entryId, "zh-Hans-CN", signal)).resolves.toEqual(
    overview,
  )

  expect(fetchMock).toHaveBeenCalledWith(
    `/api/v1/entries/${entryId}/ai?translationLocale=zh-Hans-CN`,
    expect.objectContaining({ signal, credentials: "same-origin" }),
  )
})

it.each([200, 201])("accepts %s enqueue responses and sends CSRF", async (status) => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(job, status))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal
  const request: EnqueueAiJobRequest = {
    operation: "SUMMARIZE",
    targetLocale: null,
    idempotencyKey: "reader:summary:1",
  }

  await expect(
    enqueueAiJob(entryId, "csrf-memory", request, signal),
  ).resolves.toEqual(job)

  const [path, init] = fetchMock.mock.calls[0] ?? []
  expect(path).toBe(`/api/v1/entries/${entryId}/ai/jobs`)
  expect(init?.method).toBe("POST")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual(request)
})

it("gets job and result, and accepts current-snapshot retry", async () => {
  const retryJob = { ...job, jobId: "00000000-0000-4000-8000-000000000402" }
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(job))
    .mockResolvedValueOnce(jsonResponse(artifact))
    .mockResolvedValueOnce(jsonResponse(retryJob, 201))
  vi.stubGlobal("fetch", fetchMock)

  await expect(getAiJob(jobId)).resolves.toEqual(job)
  await expect(getAiJobResult(jobId)).resolves.toEqual(artifact)
  await expect(
    retryAiJob(jobId, "csrf-memory", { idempotencyKey: "reader:retry:1" }),
  ).resolves.toEqual(retryJob)

  expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
    `/api/v1/ai/jobs/${jobId}`,
    `/api/v1/ai/jobs/${jobId}/result`,
    `/api/v1/ai/jobs/${jobId}/retry`,
  ])
  expect(new Headers(fetchMock.mock.calls[2]?.[1]?.headers).get("x-csrf-token")).toBe(
    "csrf-memory",
  )
})

it.each([
  ["config", () => getAiConfig(), { ...configEnvelope, pluginState: "STARTED" }],
  ["overview", () => getEntryAiOverview(entryId), { ...overview, contentHash: "private" }],
  ["job", () => getAiJob(jobId), { ...job, attempts: 4 }],
  ["artifact", () => getAiJobResult(jobId), { ...artifact, payloadJson: "private" }],
])("rejects malformed %s success responses", async (_name, request, body) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(body)))
  await expect(request()).rejects.toMatchObject({
    payload: { code: "INVALID_RESPONSE" },
  })
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
