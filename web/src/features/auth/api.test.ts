import { afterEach, expect, it, vi } from "vitest"

import { login, logout } from "./api"

afterEach(() => vi.unstubAllGlobals())

it("rejects malformed 2xx login JSON with a safe client error", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ user: null, csrfToken: 42 }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ),
  )

  await expect(login({ login: "Reader", password: "secret" })).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE", message: "Invalid server response" },
  })
})

it.each([
  ["JSON body", new Response(JSON.stringify({ status: "OK" }), { status: 200 })],
  ["empty non-204 body", new Response(null, { status: 202 })],
])("rejects a logout success with an unexpected %s", async (_name, response) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response))

  await expect(logout("csrf-token")).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE", message: "Invalid server response" },
  })
})

it("accepts the exact 204 no-body logout contract", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response(null, { status: 204 })),
  )

  await expect(logout("csrf-token")).resolves.toBeUndefined()
})
