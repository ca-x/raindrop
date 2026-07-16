import { afterEach, expect, it, vi } from "vitest"

import { checkDatabase, completeSetup } from "./api"
import { initialSetupValues } from "./model"

afterEach(() => vi.unstubAllGlobals())

it.each([
  ["database check", () => checkDatabase(initialSetupValues)],
  [
    "setup completion",
    () =>
      completeSetup({
        ...initialSetupValues,
        username: "Reader",
        password: "correct horse battery staple",
      }),
  ],
])("rejects a malformed 2xx %s response safely", async (_name, request) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ status: "UNKNOWN" })))

  await expect(request()).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE", message: "Invalid server response" },
  })
})

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
