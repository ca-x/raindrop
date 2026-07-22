import { afterEach, expect, it, vi } from "vitest"

import {
  translateEntryProgressively,
  translateSelectedText,
} from "./translation"

afterEach(() => vi.unstubAllGlobals())

const result = {
  translatedText: "敏捷的棕色狐狸。",
  providerLabel: "DeepLX",
  detectedSourceLocale: "en",
  targetLocale: "zh-CN",
}

it("translates selected text with CSRF, cancellation, and strict validation", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(result))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await expect(
    translateSelectedText("The quick brown fox.", "csrf-memory", signal),
  ).resolves.toEqual(result)

  const [path, init] = fetchMock.mock.calls[0] ?? []
  expect(path).toBe("/api/v3/plugins/translation/translate")
  expect(init?.method).toBe("POST")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual({
    text: "The quick brown fox.",
  })
})

it("rejects malformed selected-text translation output", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(jsonResponse({ ...result, internalPrompt: "private" })),
  )

  await expect(
    translateSelectedText("The quick brown fox.", "csrf-memory"),
  ).rejects.toMatchObject({
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("parses split progressive translation events in order", async () => {
  const events = [
    progressEvent("STARTED", { targetLocale: "zh-CN", totalSegments: 1 }),
    progressEvent("TITLE", {
      title: "已翻译标题",
      providerLabel: "DeepLX",
      detectedSourceLocale: "en",
      targetLocale: "zh-CN",
      totalSegments: 1,
    }),
    progressEvent("SEGMENT", {
      segment: {
        index: 0,
        originalText: "First paragraph",
        translatedText: "第一段",
      },
      completedSegments: 1,
      totalSegments: 1,
    }),
    progressEvent("COMPLETED", {
      completedSegments: 1,
      totalSegments: 1,
    }),
  ]
  const encoded = events.map((event) => `${JSON.stringify(event)}\n`).join("")
  const midpoint = Math.floor(encoded.length / 2)
  const fetchMock = vi.fn().mockResolvedValue(
    streamResponse([encoded.slice(0, midpoint), encoded.slice(midpoint)]),
  )
  vi.stubGlobal("fetch", fetchMock)
  const onProgress = vi.fn()

  await expect(
    translateEntryProgressively(
      "00000000-0000-4000-8000-000000000301",
      "csrf-memory",
      onProgress,
    ),
  ).resolves.toBeUndefined()
  expect(onProgress.mock.calls.map(([event]) => event.kind)).toEqual([
    "STARTED",
    "TITLE",
    "SEGMENT",
    "COMPLETED",
  ])
  const [path, init] = fetchMock.mock.calls[0] ?? []
  expect(path).toContain("/translate/progressive")
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
})

it("rejects a progressive stream with invalid event order", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      streamResponse([
        `${JSON.stringify(progressEvent("COMPLETED", {
          completedSegments: 0,
          totalSegments: 0,
        }))}\n`,
      ]),
    ),
  )

  await expect(
    translateEntryProgressively(
      "00000000-0000-4000-8000-000000000301",
      "csrf-memory",
      vi.fn(),
    ),
  ).rejects.toMatchObject({ payload: { code: "INVALID_RESPONSE" } })
})

it("rejects progressive segments whose indexes do not match stream order", async () => {
  const events = [
    progressEvent("STARTED", { targetLocale: "zh-CN", totalSegments: 1 }),
    progressEvent("TITLE", {
      title: "已翻译标题",
      providerLabel: "DeepLX",
      targetLocale: "zh-CN",
      totalSegments: 1,
    }),
    progressEvent("SEGMENT", {
      segment: {
        index: 1,
        originalText: "First paragraph",
        translatedText: "第一段",
      },
      completedSegments: 1,
      totalSegments: 1,
    }),
  ]
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      streamResponse(events.map((event) => `${JSON.stringify(event)}\n`)),
    ),
  )

  await expect(
    translateEntryProgressively(
      "00000000-0000-4000-8000-000000000301",
      "csrf-memory",
      vi.fn(),
    ),
  ).rejects.toMatchObject({ payload: { code: "INVALID_RESPONSE" } })
})

it("rejects completion before every progressive segment arrives", async () => {
  const events = [
    progressEvent("STARTED", { targetLocale: "zh-CN", totalSegments: 1 }),
    progressEvent("TITLE", {
      title: "已翻译标题",
      providerLabel: "DeepLX",
      targetLocale: "zh-CN",
      totalSegments: 1,
    }),
    progressEvent("COMPLETED", {
      completedSegments: 1,
      totalSegments: 1,
    }),
  ]
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      streamResponse(events.map((event) => `${JSON.stringify(event)}\n`)),
    ),
  )

  await expect(
    translateEntryProgressively(
      "00000000-0000-4000-8000-000000000301",
      "csrf-memory",
      vi.fn(),
    ),
  ).rejects.toMatchObject({ payload: { code: "INVALID_RESPONSE" } })
})

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}

function progressEvent(kind: string, patch: Record<string, unknown>) {
  return {
    kind,
    title: null,
    segment: null,
    providerLabel: null,
    detectedSourceLocale: null,
    targetLocale: null,
    completedSegments: 0,
    totalSegments: 0,
    error: null,
    ...patch,
  }
}

function streamResponse(chunks: string[]): Response {
  const encoder = new TextEncoder()
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
        controller.close()
      },
    }),
    {
      status: 200,
      headers: { "content-type": "application/x-ndjson; charset=utf-8" },
    },
  )
}
