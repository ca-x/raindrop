import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { App } from "../../app/App"
import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { setTestViewport } from "../../test/setup"

const fetchMock = vi.fn<typeof fetch>()

describe("Setup recovery and mobile targets", () => {
  beforeEach(() => {
    activateLocale("en")
    vi.stubGlobal("fetch", fetchMock)
    fetchMock.mockReset()
  })

  afterEach(() => vi.unstubAllGlobals())

  it("moves to normal login when completion succeeds but automatic login fails", async () => {
    const user = userEvent.setup()
    fetchMock
      .mockResolvedValueOnce(
        jsonResponse({ status: "SETUP_REQUIRED", version: "0.1.0", setupMode: "FULL" }),
      )
      .mockResolvedValueOnce(jsonResponse({ status: "OK", databaseKind: "SQLITE" }))
      .mockResolvedValueOnce(jsonResponse({ status: "READY", user: publicUser }))
      .mockResolvedValueOnce(jsonResponse({ error: { code: "INVALID_CREDENTIALS" } }, 401))
      .mockResolvedValueOnce(jsonResponse(sessionResponse))
    mockReaderWorkspace()
    renderApp()

    await user.type(await screen.findByLabelText(/Setup token/), "rd_setup_recovery")
    await user.click(screen.getByRole("button", { name: "Check database and continue" }))
    await user.type(await screen.findByLabelText(/Username/), "Reader")
    await user.type(screen.getByLabelText(/Password/), "correct horse battery staple")
    await user.click(screen.getByRole("button", { name: "Complete setup" }))

    expect(await screen.findByRole("heading", { name: "Welcome back" })).toBeVisible()
    expect(screen.queryByText("Setup could not be completed")).not.toBeInTheDocument()
    await user.type(screen.getByLabelText(/Username or email/), "Reader")
    await user.type(screen.getByLabelText(/Password/), "correct horse battery staple")
    await user.click(screen.getByRole("button", { name: "Sign in" }))

    expect(await screen.findByRole("heading", { name: "No entries here" })).toBeVisible()
    expect(paths("/api/v1/setup/complete")).toHaveLength(1)
    expect(paths("/api/v1/auth/login")).toHaveLength(2)
  })

  it.each([
    [360, 800],
    [390, 844],
  ])("exposes 44px setup targets at %ix%i", async (width, height) => {
    setTestViewport(width, height)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ status: "SETUP_REQUIRED", version: "0.1.0", setupMode: "FULL" }),
    )
    renderApp()

    const token = await screen.findByLabelText(/Setup token/)
    expect(token.parentElement).toHaveStyle({ minHeight: "44px" })
    expect(screen.getByLabelText(/Database URL/).parentElement).toHaveStyle({
      minHeight: "44px",
    })
    for (const kind of ["sqlite", "postgres", "mysql"]) {
      expect(screen.getByTestId(`database-kind-${kind}`)).toHaveStyle({ minHeight: "44px" })
    }
    expect(screen.getByRole("radiogroup", { name: "Language" })).toHaveStyle({
      "--size-element-md": "48px",
    })
    expect(screen.getByRole("button", { name: "Check database and continue" })).toHaveStyle({
      minHeight: "44px",
    })
  })
})

function renderApp() {
  render(<Providers><App /></Providers>)
}

function paths(path: string) {
  return fetchMock.mock.calls.filter(([calledPath]) => calledPath === path)
}

function mockReaderWorkspace() {
  fetchMock
    .mockResolvedValueOnce(jsonResponse({ items: [] }))
    .mockResolvedValueOnce(jsonResponse({ items: [], nextCursor: null }))
    .mockResolvedValueOnce(jsonResponse({ items: [], nextCursor: null, snapshotGeneration: 1 }))
}

const publicUser = {
  id: "98d3278e-b1bf-4eca-8158-e55c226f9965",
  username: "Reader",
  email: null,
  isDisabled: false,
  roles: ["ADMIN", "USER"],
}

const sessionResponse = {
  user: publicUser,
  csrfToken: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  expiresAt: "2026-08-15T08:00:00Z",
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
