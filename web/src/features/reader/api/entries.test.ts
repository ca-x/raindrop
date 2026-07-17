import { afterEach, expect, it, vi } from "vitest"

import { getEntry, listEntries, patchEntryState } from "./entries"
import type { PatchEntryStateRequest } from "./reader.generated"

// @ts-expect-error The OpenAPI anyOf contract requires at least one state field.
const emptyPatchIsRejected: PatchEntryStateRequest = {}
void emptyPatchIsRejected

afterEach(() => vi.unstubAllGlobals())

const entryId = "00000000-0000-4000-8000-000000000301"
const feedId = "00000000-0000-4000-8000-000000000101"
const entry = {
  entryId,
  feedId,
  feedTitle: "Example Feed",
  siteUrl: "https://example.com/",
  title: "Entry title",
  author: "Reader",
  summary: "Summary",
  canonicalUrl: "https://example.com/article",
  publishedAtUs: 1_784_246_400_000_000,
  sortAtUs: 1_784_246_400_000_000,
  isRead: false,
  isStarred: false,
}
const entryPage = { items: [entry], nextCursor: null, snapshotGeneration: 1 }
const detail = { ...entry, contentHtml: "<p>Safe</p>", inertImages: [], enclosures: [] }
const entryState = { entryId, isRead: true, isStarred: false }

it("lists entries with artifact-backed state and query parameters", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(entryPage))
  vi.stubGlobal("fetch", fetchMock)

  await expect(
    listEntries({ state: "STARRED", feedId, limit: 20, cursor: "next" }),
  ).resolves.toEqual(entryPage)
  expect(fetchMock.mock.calls[0]?.[0]).toBe(
    `/api/v1/entries?cursor=next&limit=20&feedId=${feedId}&state=STARRED`,
  )
})

it.each([
  ["non-array page", () => listEntries(), { ...entryPage, items: {} }],
  ["missing page field", () => listEntries(), { items: [entry], nextCursor: null }],
  ["wrong detail content", () => getEntry(entryId), { ...detail, contentHtml: 7 }],
  ["internal detail field", () => getEntry(entryId), { ...detail, storageKey: "secret" }],
  [
    "wrong state field",
    () => patchEntryState(entryId, { isRead: true }, "csrf"),
    { entryId, isRead: "yes", isStarred: false },
  ],
])("rejects a malformed 2xx %s response", async (_name, request, body) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(body)))
  await expect(request()).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE", message: "Invalid server response" },
  })
})

it("uses in-memory CSRF and AbortSignal for entry state PATCH", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(entryState))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await expect(
    patchEntryState(entryId, { isRead: true }, "csrf-memory", signal),
  ).resolves.toEqual(entryState)

  const [path, init] = fetchMock.mock.calls[0]!
  expect(path).toBe(`/api/v1/entries/${entryId}/state`)
  expect(init?.method).toBe("PATCH")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual({ isRead: true })
})

it("passes AbortSignal through entry list and detail requests", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(entryPage))
    .mockResolvedValueOnce(jsonResponse(detail))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await listEntries({ signal })
  await getEntry(entryId, signal)

  expect(fetchMock.mock.calls.every((call) => call[1]?.signal === signal)).toBe(true)
})

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
