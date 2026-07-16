import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { App } from "../../app/App"
import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { setTestViewport } from "../../test/setup"

const fetchMock = vi.fn<typeof fetch>()

describe("Local authentication", () => {
  beforeEach(() => {
    activateLocale("en")
    localStorage.clear()
    vi.stubGlobal("fetch", fetchMock)
    fetchMock.mockReset()
  })

  afterEach(() => vi.unstubAllGlobals())

  it("shows login loading and a stable authentication error", async () => {
    const user = userEvent.setup()
    const loginRequest = deferred<Response>()
    mockLoginBootstrap()
    fetchMock.mockReturnValueOnce(loginRequest.promise)
    renderApp()

    expect(await screen.findByRole("img", { name: "Raindrop" })).toHaveAttribute("width", "32")
    await fillLogin(user)
    const submit = screen.getByRole("button", { name: "Sign in" })
    await user.click(submit)
    expect(submit).toBeDisabled()
    expect(submit).toHaveAttribute("aria-busy", "true")

    loginRequest.resolve(
      jsonResponse({ error: { code: "INVALID_CREDENTIALS", message: "Invalid" } }, 401),
    )
    expect(await screen.findByText("Sign-in failed")).toBeVisible()
    expect(screen.getByDisplayValue("reader@example.com")).toBeVisible()
  })

  it("signs in and keeps the CSRF token only in application state", async () => {
    const user = userEvent.setup()
    mockLoginBootstrap()
    fetchMock.mockResolvedValueOnce(jsonResponse(sessionResponse))
    renderApp()

    await fillLogin(user)
    await user.click(screen.getByRole("button", { name: "Sign in" }))

    expect(await screen.findByRole("heading", { name: "Your reading space is ready" })).toBeVisible()
    expect(screen.queryByRole("img", { name: "Raindrop" })).not.toBeInTheDocument()
    expect(screen.getByText("Raindrop")).toBeVisible()
    expect(localStorage).toHaveLength(0)
    expect(fetchMock).toHaveBeenLastCalledWith(
      "/api/v1/auth/login",
      expect.objectContaining({ credentials: "same-origin" }),
    )
  })

  it("turns a malformed login success body into the stable error state", async () => {
    const user = userEvent.setup()
    mockLoginBootstrap()
    fetchMock.mockResolvedValueOnce(jsonResponse({ user: null, csrfToken: 42 }))
    renderApp()

    await fillLogin(user)
    await user.click(screen.getByRole("button", { name: "Sign in" }))

    expect(await screen.findByText("Sign-in failed")).toBeVisible()
    expect(screen.queryByText("invalid login response")).not.toBeInTheDocument()
  })

  it.each([
    { width: 360, height: 800, mode: "mobile" },
    { width: 390, height: 844, mode: "mobile" },
    { width: 1280, height: 800, mode: "desktop" },
  ])("exposes logout and exits at $width px", async ({ width, height, mode }) => {
    const user = userEvent.setup()
    setTestViewport(width, height)
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ status: "READY", version: "0.1.0" }))
      .mockResolvedValueOnce(jsonResponse(sessionResponse))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
    renderApp()

    expect(await screen.findByRole("heading", { name: "Your reading space is ready" })).toBeVisible()
    const directLogout = screen.getByRole("button", { name: "Sign out" })
    expect(directLogout).toBeVisible()

    if (mode === "mobile") {
      expect(screen.getByTestId("mobile-ready-page")).toBeVisible()
      await user.click(screen.getByRole("button", { name: "Open menu" }))
      const dialog = await screen.findByRole("dialog", { name: "Open menu" })
      expect(dialog).toHaveStyle({ "--size-element-md": "44px" })
      const menuLogout = within(dialog).getByRole("button", { name: "Sign out" })
      expect(menuLogout).toHaveStyle({ minHeight: "44px", minWidth: "44px" })
      await user.click(menuLogout)
    } else {
      expect(screen.queryByTestId("mobile-ready-page")).not.toBeInTheDocument()
      expect(screen.queryByRole("button", { name: "Open menu" })).not.toBeInTheDocument()
      await user.click(directLogout)
    }

    expect(await screen.findByRole("heading", { name: "Welcome back" })).toBeVisible()
    const [path, init] = fetchMock.mock.calls.at(-1) ?? []
    expect(path).toBe("/api/v1/auth/logout")
    expect(new Headers(init?.headers).get("x-csrf-token")).toBe(sessionResponse.csrfToken)
  })

  it("exposes 44px login fields, locale choices, and submit at 360x800", async () => {
    setTestViewport(360, 800)
    mockLoginBootstrap(false)
    renderApp()

    const identifier = await screen.findByLabelText(/Username or email/)
    expect(identifier.parentElement).toHaveStyle({ minHeight: "44px" })
    expect(screen.getByLabelText(/Password/).parentElement).toHaveStyle({ minHeight: "44px" })
    expect(screen.getByRole("radiogroup", { name: "Language" })).toHaveStyle({
      "--size-element-md": "48px",
    })
    expect(screen.getByRole("button", { name: "Sign in" })).toHaveStyle({ minHeight: "44px" })
  })
})

function renderApp() {
  render(<Providers><App /></Providers>)
}

function mockLoginBootstrap(resetViewport = true) {
  if (resetViewport) setTestViewport(1280, 800)
  fetchMock
    .mockResolvedValueOnce(jsonResponse({ status: "READY", version: "0.1.0" }))
    .mockResolvedValueOnce(jsonResponse({ error: { code: "AUTHENTICATION_REQUIRED" } }, 401))
}

async function fillLogin(user: ReturnType<typeof userEvent.setup>) {
  await user.type(await screen.findByLabelText(/Username or email/), "reader@example.com")
  await user.type(screen.getByLabelText(/Password/), "correct horse battery staple")
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

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
