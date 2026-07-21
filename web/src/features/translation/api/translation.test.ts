import { afterEach, expect, it, vi } from "vitest"

import { translateSelectedText } from "./translation"

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
  expect(path).toBe("/api/v2/plugins/translation/translate")
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

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
