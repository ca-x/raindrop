import { afterEach, expect, it, vi } from "vitest"

import { checkDatabase, completeAdminSetup, completeSetup } from "./api"
import { initialSetupValues } from "./model"

afterEach(() => vi.unstubAllGlobals())

it("accepts the backend PostgreSQL database-kind contract", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      jsonResponse({ status: "OK", databaseKind: "POSTGRESQL" }),
    ),
  )

  await expect(checkDatabase(initialSetupValues)).resolves.toEqual({
    status: "OK",
    databaseKind: "POSTGRESQL",
  })
})

it("rejects unknown database-kind values", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(jsonResponse({ status: "OK", databaseKind: "ORACLE" })),
  )

  await expect(checkDatabase(initialSetupValues)).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("completes managed administrator setup without sending database configuration", async () => {
  const fetchMock = vi.fn().mockResolvedValue(
    jsonResponse({
      status: "READY",
      user: {
        id: "98d3278e-b1bf-4eca-8158-e55c226f9965",
        username: "Reader",
        email: null,
        isDisabled: false,
        roles: ["ADMIN", "USER"],
      },
    }),
  )
  vi.stubGlobal("fetch", fetchMock)
  const values = {
    ...initialSetupValues,
    token: "rd_setup_managed_token",
    databaseUrl: "postgres://must-not-be-sent.example/raindrop",
    username: "Reader",
    password: "correct horse battery staple",
  }

  await completeAdminSetup(values)

  const [path, init] = fetchMock.mock.calls[0]!
  expect(path).toBe("/api/v1/setup/admin")
  expect(new Headers(init?.headers).get("x-setup-token")).toBe(values.token)
  expect(JSON.parse(String(init?.body))).toEqual({
    username: "Reader",
    password: "correct horse battery staple",
    email: null,
  })
})

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
