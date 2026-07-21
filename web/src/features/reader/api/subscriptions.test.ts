import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"

import { afterEach, expect, it, vi } from "vitest"

import {
  createSubscription,
  deleteSubscription,
  getSubscription,
  listSubscriptions,
  refreshSubscription,
  updateSubscription,
} from "./subscriptions"
import type { UpdateSubscriptionRequest } from "./subscription.generated"

// @ts-expect-error The OpenAPI minProperties contract requires at least one patch field.
const emptySubscriptionPatch: UpdateSubscriptionRequest = {}
void emptySubscriptionPatch

const temporaryDirectories: string[] = []

afterEach(() => {
  vi.unstubAllGlobals()
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true })
  }
})

const requestId = "00000000-0000-4000-8000-000000000901"
const subscription = {
  subscriptionId: "00000000-0000-4000-8000-000000000201",
  feedId: "00000000-0000-4000-8000-000000000101",
  categoryId: null,
  titleOverride: null,
  position: 0,
  title: "Example Feed",
  feedUrl: "https://example.com/feed.xml",
  siteUrl: "https://example.com/",
  unreadCount: 3,
  refresh: null,
}
const subscriptionPage = { items: [subscription], nextCursor: null }
const refresh = {
  operationId: "00000000-0000-4000-8000-000000000801",
  state: "PENDING",
  pendingState: "QUEUED",
  newCount: 0,
  updatedCount: 0,
  droppedCount: 0,
  entryIssues: [],
  generation: null,
  errorCode: null,
  retryAt: null,
  lastSuccessAt: null,
  queuedAt: "2026-07-18T02:00:00.000000Z",
  startedAt: null,
  completedAt: null,
}

it("lists subscriptions with validated paging and query parameters", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(subscriptionPage))
  vi.stubGlobal("fetch", fetchMock)

  await expect(listSubscriptions({ cursor: "next", limit: 25 })).resolves.toEqual(
    subscriptionPage,
  )
  expect(fetchMock.mock.calls[0]?.[0]).toBe(
    "/api/v1/subscriptions?cursor=next&limit=25",
  )
})

it.each([
  ["non-array page", () => listSubscriptions(), { items: {}, nextCursor: null }],
  [
    "wrong subscription field",
    () => getSubscription(subscription.subscriptionId),
    { ...subscription, unreadCount: "3" },
  ],
  [
    "missing create field",
    () => createSubscription({ url: "https://example.com/feed.xml" }, "csrf"),
    { subscription },
  ],
  [
    "unknown refresh state",
    () => refreshSubscription(subscription.subscriptionId, { requestId }, "csrf"),
    { ...refresh, state: "QUEUED" },
  ],
  [
    "missing patch projection field",
    () => updateSubscription(subscription.subscriptionId, { position: 1 }, "csrf"),
    { ...subscription, categoryId: undefined },
  ],
])("rejects a malformed 2xx %s response", async (_name, request, body) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(body)))
  await expect(request()).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("sends in-memory CSRF tokens for every subscription mutation", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse({ created: true, subscription }))
    .mockResolvedValueOnce(jsonResponse({ ...subscription, position: 1024 }))
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
    .mockResolvedValueOnce(jsonResponse(refresh))
  vi.stubGlobal("fetch", fetchMock)

  await createSubscription({ url: "https://example.com/feed.xml" }, "csrf-memory")
  await updateSubscription(
    subscription.subscriptionId,
    { categoryId: null, titleOverride: "Focused", position: 1024 },
    "csrf-memory",
  )
  await deleteSubscription(subscription.subscriptionId, "csrf-memory")
  await refreshSubscription(subscription.subscriptionId, { requestId }, "csrf-memory")

  for (const call of fetchMock.mock.calls) {
    expect(new Headers(call[1]?.headers).get("x-csrf-token")).toBe("csrf-memory")
  }
  expect(fetchMock.mock.calls.map((call) => call[1]?.method)).toEqual([
    "POST",
    "PATCH",
    "DELETE",
    "POST",
  ])
  expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toEqual({
    categoryId: null,
    titleOverride: "Focused",
    position: 1024,
  })
})

it("passes AbortSignal through every subscription request", async () => {
  const fetchMock = vi.fn(async (path: string | URL | Request, init?: RequestInit) => {
    const url = String(path)
    if (init?.method === "DELETE") return new Response(null, { status: 204 })
    if (init?.method === "PATCH") return jsonResponse({ ...subscription, position: 1024 })
    if (url.endsWith("/refresh")) return jsonResponse(refresh)
    if (url === "/api/v1/subscriptions" && init?.method === "POST") {
      return jsonResponse({ created: true, subscription })
    }
    return jsonResponse(url === "/api/v1/subscriptions" ? subscriptionPage : subscription)
  })
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await listSubscriptions({ signal })
  await getSubscription(subscription.subscriptionId, signal)
  await createSubscription({ url: "https://example.com/feed.xml" }, "csrf", signal)
  await updateSubscription(subscription.subscriptionId, { position: 1024 }, "csrf", signal)
  await deleteSubscription(subscription.subscriptionId, "csrf", signal)
  await refreshSubscription(subscription.subscriptionId, { requestId }, "csrf", signal)

  expect(fetchMock.mock.calls).toHaveLength(6)
  expect(fetchMock.mock.calls.every((call) => call[1]?.signal === signal)).toBe(true)
})

it("rejects a non-empty delete success response", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ deleted: true })))
  await expect(deleteSubscription(subscription.subscriptionId, "csrf")).rejects.toMatchObject({
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("generates both artifacts deterministically and detects missing or edited output", () => {
  const outputRoot = mkdtempSync(join(tmpdir(), "raindrop-reader-types-"))
  temporaryDirectories.push(outputRoot)

  const missing = runGenerator("--check", "--output-root", outputRoot)
  expect(missing.status).toBe(1)
  expect(missing.stderr).toContain("missing generated file")

  const generated = runGenerator("--output-root", outputRoot)
  expect(generated.status, generated.stderr).toBe(0)
  expect(generated.stdout).toContain("subscription.generated.ts")
  expect(generated.stdout).toContain("reader.generated.ts")
  expect(generated.stdout).toContain("organization.generated.ts")

  const clean = runGenerator("--check", "--output-root", outputRoot)
  expect(clean.status, clean.stderr).toBe(0)

  const subscriptionPath = join(
    outputRoot,
    "src/features/reader/api/subscription.generated.ts",
  )
  const original = readFileSync(subscriptionPath, "utf8")
  expect(original).toContain("title: string")
  expect(original).toContain("categoryId: string | null")
  expect(original).toContain("export type UpdateSubscriptionRequest = {")
  expect(original).toContain("| { position: number }")
  expect(original).toContain("export type SubscriptionResponse = Subscription")
  expect(original).toContain("export type SubscriptionPageResponse = SubscriptionPage")
  expect(original).toContain("export type RefreshResponse = Refresh")
  expect(original).toContain("export type ApiErrorEnvelope = ErrorEnvelope")
  expect(original).toContain("export function isSubscriptionPageResponse")
  expect(original).toContain("items: Subscription[]")
  expect(original).toContain("feedUrl: string")
  expect(original).toContain("siteUrl: string | null")
  expect(original).toContain("fields?: Record<string, string>")
  writeFileSync(subscriptionPath, original.replace("title: string", "title: number"))

  const edited = runGenerator("--check", "--output-root", outputRoot)
  expect(edited.status).toBe(1)
  expect(edited.stderr).toContain("subscription.generated.ts")

  writeFileSync(subscriptionPath, original)
  const readerPath = join(outputRoot, "src/features/reader/api/reader.generated.ts")
  const originalReader = readFileSync(readerPath, "utf8")
  expect(originalReader).toContain("export type PatchEntryStateRequest = {")
  expect(originalReader).toContain("} & (")
  expect(originalReader).toContain("| { isRead: boolean }")
  expect(originalReader).toContain("| { isStarred: boolean }")
  writeFileSync(
    readerPath,
    originalReader.replace("contentHtml: string", "contentHtml: number"),
  )
  const editedReader = runGenerator("--check", "--output-root", outputRoot)
  expect(editedReader.status).toBe(1)
  expect(editedReader.stderr).toContain("reader.generated.ts")

  writeFileSync(readerPath, originalReader)
  const organizationPath = join(
    outputRoot,
    "src/features/reader/api/organization.generated.ts",
  )
  const originalOrganization = readFileSync(organizationPath, "utf8")
  expect(originalOrganization).toContain("export interface Category")
  expect(originalOrganization).toContain("export type UpdateCategoryRequest = {")
  expect(originalOrganization).toContain("value[\"items\"].length <= 250")
  writeFileSync(
    organizationPath,
    originalOrganization.replace("title: string", "title: number"),
  )
  const editedOrganization = runGenerator("--check", "--output-root", outputRoot)
  expect(editedOrganization.status).toBe(1)
  expect(editedOrganization.stderr).toContain("organization.generated.ts")
})

function runGenerator(...args: string[]) {
  const result = spawnSync(
    process.execPath,
    ["scripts/generate-reader-types.mjs", ...args],
    {
      cwd: process.cwd(),
      encoding: "utf8",
    },
  )
  return {
    status: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
  }
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
