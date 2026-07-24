import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { activateLocale } from "../shared/i18n/i18n"
import { App } from "./App"
import { Providers } from "./Providers"

const fetchMock = vi.fn<typeof fetch>()

describe("App bootstrap routing", () => {
  beforeEach(() => {
    window.history.replaceState(null, "", "/")
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
    await waitFor(() => expect(window.location.pathname).toBe("/login"))
  })

  it("returns to a requested reader route after authentication", async () => {
    const user = userEvent.setup()
    activateLocale("en")
    window.history.replaceState(null, "", "/reader/all")
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ status: "READY", version: "0.4.2" }))
      .mockResolvedValueOnce(jsonResponse({ error: { code: "AUTHENTICATION_REQUIRED" } }, 401))
      .mockResolvedValueOnce(jsonResponse(sessionResponse))
      .mockResolvedValue(jsonResponse({ items: [] }))

    render(
      <Providers>
        <App />
      </Providers>,
    )

    await screen.findByRole("heading", { name: "Welcome back" })
    await waitFor(() => expect(window.location.pathname).toBe("/login"))
    await user.type(screen.getByLabelText(/Username or email/), "reader@example.com")
    await user.type(screen.getByLabelText(/Password/), "correct horse battery staple")
    await user.click(screen.getByRole("button", { name: "Sign in" }))

    await waitFor(() => expect(window.location.pathname).toBe("/reader/all"))
  })
})

const sessionResponse = {
  user: {
    id: "98d3278e-b1bf-4eca-8158-e55c226f9965",
    username: "Reader",
    email: "reader@example.com",
    isDisabled: false,
    roles: ["ADMIN", "USER"],
  },
  csrfToken: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  expiresAt: "2026-08-15T08:00:00Z",
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
