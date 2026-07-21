import { act, renderHook, waitFor } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import { useEntryTranslationController } from "./useEntryTranslationController"

afterEach(() => vi.unstubAllGlobals())

const firstEntryId = "00000000-0000-4000-8000-000000000301"
const secondEntryId = "00000000-0000-4000-8000-000000000302"
const translated = {
  translatedText: "选中的段落。",
  providerLabel: "DeepLX",
  detectedSourceLocale: "en",
  targetLocale: "zh-CN",
}

it("tracks selected-text translation independently from article translation", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(translated)))
  const { result, rerender } = renderHook(
    ({ entryId }) =>
      useEntryTranslationController(entryId, "csrf-memory", vi.fn()),
    { initialProps: { entryId: firstEntryId } },
  )

  await act(async () => {
    await expect(result.current.translateSelection("Selected paragraph.")).resolves.toBe(
      true,
    )
  })
  expect(result.current.selectionResult).toEqual(translated)
  expect(result.current.result).toBeNull()

  rerender({ entryId: secondEntryId })
  await waitFor(() => expect(result.current.selectionResult).toBeNull())
})

it("aborts a pending selected-text request when the reader changes entry", async () => {
  let observedSignal: AbortSignal | undefined
  vi.stubGlobal(
    "fetch",
    vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
      observedSignal = init?.signal ?? undefined
      return new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener(
          "abort",
          () => reject(new DOMException("Aborted", "AbortError")),
          { once: true },
        )
      })
    }),
  )
  const { result, rerender } = renderHook(
    ({ entryId }) =>
      useEntryTranslationController(entryId, "csrf-memory", vi.fn()),
    { initialProps: { entryId: firstEntryId } },
  )

  let request!: Promise<boolean>
  act(() => {
    request = result.current.translateSelection("Selected paragraph.")
  })
  await waitFor(() => expect(result.current.isTranslatingSelection).toBe(true))

  rerender({ entryId: secondEntryId })
  expect(observedSignal?.aborted).toBe(true)
  await act(async () => expect(request).resolves.toBe(false))
  await waitFor(() => expect(result.current.isTranslatingSelection).toBe(false))
})

it("does not erase an article failure when contextual actions close", async () => {
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("offline")))
  const { result } = renderHook(() =>
    useEntryTranslationController(firstEntryId, "csrf-memory", vi.fn()),
  )

  await act(async () => {
    await expect(result.current.translate()).resolves.toBe(false)
  })
  expect(result.current.articleError).toBe("TRANSLATE")

  act(() => result.current.cancelContextActions())
  expect(result.current.articleError).toBe("TRANSLATE")
})

it("ignores a cancelled lookup response that arrives after selection translation", async () => {
  const lookupResponse = deferred<Response>()
  const selectionResponse = deferred<Response>()
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) =>
      String(input).endsWith("/lookup")
        ? lookupResponse.promise
        : selectionResponse.promise,
    ),
  )
  const { result } = renderHook(() =>
    useEntryTranslationController(firstEntryId, "csrf-memory", vi.fn()),
  )

  let lookupRequest!: Promise<boolean>
  act(() => {
    lookupRequest = result.current.lookup("fox")
  })
  await waitFor(() => expect(result.current.isLookingUp).toBe(true))

  let selectionRequest!: Promise<boolean>
  act(() => {
    selectionRequest = result.current.translateSelection("Selected paragraph.")
  })
  selectionResponse.resolve(jsonResponse(translated))
  await act(async () => expect(selectionRequest).resolves.toBe(true))

  lookupResponse.resolve(
    jsonResponse({
      query: "fox",
      translation: "狐狸",
      definition: null,
      examples: [],
      providerLabel: "DeepLX",
      detectedSourceLocale: "en",
      targetLocale: "zh-CN",
    }),
  )
  await act(async () => expect(lookupRequest).resolves.toBe(false))

  expect(result.current.selectionResult).toEqual(translated)
  expect(result.current.lookupResult).toBeNull()
})

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  })
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
