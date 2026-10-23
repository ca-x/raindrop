import { afterEach, expect, it, vi } from "vitest"

import { loadTamboSettings, saveTamboSettings, type TamboSettings } from "./api"

afterEach(() => vi.unstubAllGlobals())

const settings: TamboSettings = { configured: true, enabled: false, origin: "https://tambo.example", revision: 7 }

it("saves the exact optimistic revision with session credentials and CSRF", async () => {
  const fetcher = vi.fn().mockResolvedValue(Response.json({ ...settings, enabled: true, revision: 8 }))
  vi.stubGlobal("fetch", fetcher)
  expect(await saveTamboSettings(settings, true, "session-csrf")).toEqual({ ...settings, enabled: true, revision: 8 })
  const [path, request] = fetcher.mock.calls[0] as [string, RequestInit]
  expect(path).toBe("/api/v1/ai/tambo")
  expect(request.method).toBe("PUT")
  expect(request.credentials).toBe("same-origin")
  expect(new Headers(request.headers).get("x-csrf-token")).toBe("session-csrf")
  expect(JSON.parse(String(request.body))).toEqual({ expectedRevision: 7, enabled: true })
})

it("accepts persisted opt-in while the administrator has temporarily removed the service", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ configured: false, enabled: true, origin: null, revision: 0 })))
  expect((await loadTamboSettings()).enabled).toBe(true)
})

it.each([
  { ...settings, origin: "https://user:secret@tambo.example" },
  { ...settings, origin: "javascript:alert(1)" },
  { ...settings, origin: "https://tambo.example/path" },
  { ...settings, revision: -1 },
  { ...settings, revision: Number.MAX_SAFE_INTEGER + 1 },
  { ...settings, apiKey: "unexpected-secret" },
  { ...settings, configured: false },
  { enabled: false },
])("rejects invalid or secret-bearing settings: %j", async (response) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json(response)))
  await expect(loadTamboSettings()).rejects.toMatchObject({ status: 502, payload: { code: "INVALID_RESPONSE" } })
})
