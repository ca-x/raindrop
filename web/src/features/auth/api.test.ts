import { afterEach, expect, it, vi } from "vitest"

import { login } from "./api"

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
