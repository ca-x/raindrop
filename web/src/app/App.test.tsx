import { render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { activateLocale } from "../shared/i18n/i18n"
import { App } from "./App"
import { Providers } from "./Providers"

const fetchMock = vi.fn<typeof fetch>()

describe("App bootstrap routing", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", fetchMock)
    fetchMock.mockReset()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it.each([
    ["zh-CN", "开始设置 Raindrop"],
    ["en", "Set up Raindrop"],
  ] as const)("renders setup in %s", async (locale, heading) => {
    activateLocale(locale)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ status: "SETUP_REQUIRED", version: "0.1.0", setupMode: "FULL" }),
    )

    render(
      <Providers>
        <App />
      </Providers>,
    )

    expect(await screen.findByRole("heading", { name: heading })).toBeVisible()
  })

  it("renders login when setup is complete and no session exists", async () => {
    activateLocale("en")
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ status: "READY", version: "0.1.0" }))
      .mockResolvedValueOnce(jsonResponse({ error: { code: "AUTHENTICATION_REQUIRED" } }, 401))

    render(
      <Providers>
        <App />
      </Providers>,
    )

    expect(await screen.findByRole("heading", { name: "Welcome back" })).toBeVisible()
  })
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
