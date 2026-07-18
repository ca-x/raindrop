import { afterEach, expect, it, vi } from "vitest"

import {
  createCategory,
  deleteCategory,
  listCategories,
  updateCategory,
} from "./api"

afterEach(() => vi.unstubAllGlobals())

const category = {
  categoryId: "00000000-0000-4000-8000-000000000501",
  title: "Technology",
  position: 1024,
}

it("lists categories through the generated strict response validator", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ items: [category] }))
  vi.stubGlobal("fetch", fetchMock)

  await expect(listCategories()).resolves.toEqual({ items: [category] })
  expect(fetchMock).toHaveBeenCalledWith(
    "/api/v1/categories",
    expect.objectContaining({ credentials: "same-origin" }),
  )
})

it.each([
  ["non-array list", { items: {} }],
  ["unknown category field", { items: [{ ...category, normalizedTitle: "technology" }] }],
  ["invalid category position", { items: [{ ...category, position: -1 }] }],
  ["too many categories", { items: Array.from({ length: 251 }, () => category) }],
])("rejects a malformed 2xx %s response", async (_name, body) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(body)))
  await expect(listCategories()).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("sends CSRF, encoded IDs, bodies, and AbortSignal for category mutations", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(category))
    .mockResolvedValueOnce(jsonResponse({ ...category, title: "Science" }))
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await createCategory({ title: "Technology" }, "csrf-memory", signal)
  await updateCategory("category/one", { title: "Science" }, "csrf-memory", signal)
  await deleteCategory("category/one", "csrf-memory", signal)

  expect(fetchMock.mock.calls.map(([path]) => path)).toEqual([
    "/api/v1/categories",
    "/api/v1/categories/category%2Fone",
    "/api/v1/categories/category%2Fone",
  ])
  expect(fetchMock.mock.calls.map(([, init]) => init?.method)).toEqual([
    "POST",
    "PATCH",
    "DELETE",
  ])
  expect(fetchMock.mock.calls.every(([, init]) => init?.signal === signal)).toBe(true)
  expect(
    fetchMock.mock.calls.every(
      ([, init]) => new Headers(init?.headers).get("x-csrf-token") === "csrf-memory",
    ),
  ).toBe(true)
  expect(JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body))).toEqual({
    title: "Science",
  })
})

it("rejects malformed mutation responses and non-empty deletes", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ ...category, title: 7 })))
  await expect(createCategory({ title: "Technology" }, "csrf")).rejects.toMatchObject({
    payload: { code: "INVALID_RESPONSE" },
  })

  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ deleted: true })))
  await expect(deleteCategory(category.categoryId, "csrf")).rejects.toMatchObject({
    payload: { code: "INVALID_RESPONSE" },
  })
})

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
