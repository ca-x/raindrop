import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import { fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { App } from "../../app/App"
import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { setTestViewport } from "../../test/setup"

const fetchMock = vi.fn<typeof fetch>()

describe("Setup flow", () => {
  beforeEach(() => {
    setTestViewport(1280, 800)
    activateLocale("zh-CN")
    vi.stubGlobal("fetch", fetchMock)
    fetchMock.mockReset()
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ status: "SETUP_REQUIRED", version: "0.1.0", setupMode: "FULL" }),
    )
  })

  afterEach(() => vi.unstubAllGlobals())

  it("selects database URLs and exposes database-check loading", async () => {
    const user = userEvent.setup()
    const databaseCheck = deferred<Response>()
    fetchMock.mockReturnValueOnce(databaseCheck.promise)
    renderApp()

    expect(await screen.findByDisplayValue("sqlite://data/raindrop.db?mode=rwc")).toBeVisible()
    expect(screen.getByRole("img", { name: "Raindrop" })).toHaveAttribute("width", "32")
    await user.click(screen.getByRole("radio", { name: "PostgreSQL" }))
    expect(screen.getByDisplayValue("postgres://user:password@localhost/raindrop")).toBeVisible()
    await user.click(screen.getByRole("radio", { name: "MySQL" }))
    expect(screen.getByDisplayValue("mysql://user:password@localhost/raindrop")).toBeVisible()
    await user.type(screen.getByLabelText(/设置令牌/), "rd_setup_browser_token")

    const submit = screen.getByRole("button", { name: "检查数据库并继续" })
    await user.click(submit)
    expect(submit).toBeDisabled()
    expect(submit).toHaveAttribute("aria-busy", "true")
    expect(screen.getByLabelText(/设置令牌/)).toBeDisabled()
    expect(screen.getByLabelText(/数据库 URL/)).toBeDisabled()
    expect(screen.getByRole("radio", { name: "SQLite" })).toBeDisabled()
    expect(screen.getByRole("radio", { name: "中文" })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    fireEvent.submit(submit.closest("form")!)
    expect(fetchMock).toHaveBeenCalledTimes(2)

    databaseCheck.resolve(jsonResponse({ status: "OK", databaseKind: "MYSQL" }))
    expect(await screen.findByRole("heading", { name: "创建管理员" })).toBeVisible()
  })

  it("retains database input and renders the stable error banner", async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValueOnce(
      jsonResponse(
        {
          error: {
            code: "VALIDATION_ERROR",
            message: "Request validation failed",
            fields: { databaseUrl: "Database URL is invalid or unavailable" },
          },
        },
        422,
      ),
    )
    renderApp()

    await user.type(await screen.findByLabelText(/设置令牌/), "rd_setup_kept")
    await user.click(screen.getByRole("button", { name: "检查数据库并继续" }))

    expect(await screen.findByText("数据库连接失败")).toBeVisible()
    expect(screen.getByDisplayValue("rd_setup_kept")).toBeVisible()
    expect(screen.getByDisplayValue("sqlite://data/raindrop.db?mode=rwc")).toBeVisible()
  })

  it("validates the administrator before completing setup", async () => {
    const user = userEvent.setup()
    const completion = deferred<Response>()
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ status: "OK", databaseKind: "SQLITE" }))
      .mockReturnValueOnce(completion.promise)
      .mockResolvedValueOnce(jsonResponse(sessionResponse))
    mockReaderWorkspace()
    renderApp()

    await user.type(await screen.findByLabelText(/设置令牌/), "rd_setup_valid")
    await user.click(screen.getByRole("button", { name: "检查数据库并继续" }))
    await screen.findByRole("heading", { name: "创建管理员" })
    await user.type(screen.getByLabelText(/用户名/), "ab c")
    await user.type(screen.getByLabelText(/邮箱/), "invalid")
    await user.type(screen.getByLabelText(/密码/), "short")
    await user.click(screen.getByRole("button", { name: "完成设置" }))

    expect(screen.getByText("用户名需要包含 3 到 64 个非空白字符。")).toBeVisible()
    expect(screen.getByText("请输入有效的邮箱地址。")).toBeVisible()
    expect(screen.getByText("密码至少需要 12 个字节。")).toBeVisible()
    expect(fetchMock).toHaveBeenCalledTimes(2)

    await replace(user, /用户名/, "Reader")
    await replace(user, /邮箱/, "reader@example.com")
    await replace(user, /密码/, "correct horse battery staple")
    await user.click(screen.getByRole("button", { name: "完成设置" }))

    expect(screen.getByLabelText(/用户名/)).toBeDisabled()
    expect(screen.getByLabelText(/邮箱/)).toBeDisabled()
    expect(screen.getByLabelText(/密码/)).toBeDisabled()
    expect(screen.getByRole("button", { name: "返回数据库" })).toBeDisabled()
    expect(screen.getByRole("radio", { name: "中文" })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    fireEvent.submit(screen.getByRole("button", { name: "完成设置" }).closest("form")!)
    expect(fetchMock).toHaveBeenCalledTimes(3)

    completion.resolve(jsonResponse({ status: "READY", user: publicUser }))

    expect(await screen.findByRole("heading", { name: "这里没有文章" })).toBeVisible()
    const [path, init] = fetchMock.mock.calls[1]
    expect(path).toBe("/api/v1/setup/database-check")
    expect(new Headers(init?.headers).get("x-setup-token")).toBe("rd_setup_valid")
    expect(JSON.parse(String(init?.body))).toEqual({
      databaseUrl: "sqlite://data/raindrop.db?mode=rwc",
    })
    const [completePath, completeInit] = fetchMock.mock.calls[2]
    expect(completePath).toBe("/api/v1/setup/complete")
    expect(new Headers(completeInit?.headers).get("x-setup-token")).toBe(
      "rd_setup_valid",
    )
    expect(JSON.parse(String(completeInit?.body))).toEqual({
      databaseUrl: "sqlite://data/raindrop.db?mode=rwc",
      username: "Reader",
      email: "reader@example.com",
      password: "correct horse battery staple",
    })
    const [loginPath, loginInit] = fetchMock.mock.calls[3]
    expect(loginPath).toBe("/api/v1/auth/login")
    expect(loginInit).toEqual(
      expect.objectContaining({ method: "POST", credentials: "same-origin" }),
    )
    expect(new Headers(loginInit?.headers).get("content-type")).toBe("application/json")
    expect(JSON.parse(String(loginInit?.body))).toEqual({
      login: "Reader",
      password: "correct horse battery staple",
    })
  })

  it("switches locale without losing the setup state", async () => {
    const user = userEvent.setup()
    renderApp()

    await user.type(await screen.findByLabelText(/设置令牌/), "rd_setup_kept_in_memory")
    await user.click(screen.getByRole("radio", { name: "English" }))

    expect(await screen.findByRole("heading", { name: "Set up Raindrop" })).toBeVisible()
    expect(screen.getByDisplayValue("rd_setup_kept_in_memory")).toBeVisible()
  })

  it("imports generic controls directly from ASTRYX", () => {
    for (const file of ["DatabaseStep.tsx", "AdminStep.tsx", "SetupPage.tsx"]) {
      const source = readFileSync(fileURLToPath(new URL(file, import.meta.url)), "utf8")
      expect(source).toContain('from "@astryxdesign/core/')
      expect(source).not.toMatch(/shared\/components\/(Button|Dialog|Input)/)
    }
  })

  it.each([
    [1280, 800],
    [390, 844],
    [360, 800],
  ])(
    "renders managed administrator-only setup without database controls at %ix%i",
    async (width, height) => {
      setTestViewport(width, height)
      fetchMock.mockReset()
      fetchMock.mockResolvedValueOnce(
        jsonResponse({
          status: "SETUP_REQUIRED",
          version: "0.1.0",
          setupMode: "ADMIN_ONLY",
        }),
      )

      renderApp()

      expect(
        await screen.findByRole("heading", { name: "创建管理员" }),
      ).toBeVisible()
      expect(screen.getByLabelText(/设置令牌/)).toBeVisible()
      expect(screen.queryByLabelText(/数据库 URL/)).not.toBeInTheDocument()
      expect(screen.queryByRole("radio", { name: "SQLite" })).not.toBeInTheDocument()
      expect(screen.queryByRole("button", { name: "返回数据库" })).not.toBeInTheDocument()
      expect(screen.getByText("1 / 1")).toBeVisible()
    },
  )
})

function renderApp() {
  render(<Providers><App /></Providers>)
}

function mockReaderWorkspace() {
  fetchMock
    .mockResolvedValueOnce(jsonResponse({ items: [], nextCursor: null }))
    .mockResolvedValueOnce(jsonResponse({ items: [], nextCursor: null, snapshotGeneration: 1 }))
}

async function replace(user: ReturnType<typeof userEvent.setup>, label: RegExp, value: string) {
  const input = screen.getByLabelText(label)
  await user.clear(input)
  await user.type(input, value)
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

const publicUser = {
  id: "98d3278e-b1bf-4eca-8158-e55c226f9965",
  username: "Reader",
  email: "reader@example.com",
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
