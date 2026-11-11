import { afterEach, expect, it, vi } from "vitest"
import { createMcpToken, listMcpTokens, revokeMcpToken } from "./api"

afterEach(() => vi.unstubAllGlobals())

it("validates server token metadata before rendering", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ items: [{ id: "invalid" }] }))))
  await expect(listMcpTokens()).rejects.toMatchObject({ status: 502 })
})

it("does not accept a create response without valid metadata", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ token: "secret", item: {} }))))
  await expect(createMcpToken("Desktop", 90, "csrf")).rejects.toMatchObject({ status: 502 })
})

it("sends CSRF credentials only in headers and encodes token IDs", async () => {
  const request = vi.fn().mockResolvedValue(new Response(null, { status: 204 }))
  vi.stubGlobal("fetch", request)
  await revokeMcpToken("id/with space", "csrf")
  expect(request).toHaveBeenCalledWith("/api/v1/mcp/tokens/id%2Fwith%20space", expect.objectContaining({
    method: "DELETE", credentials: "same-origin",
  }))
  const headers = request.mock.calls[0]?.[1]?.headers as Headers
  expect(headers.get("x-csrf-token")).toBe("csrf")
})
