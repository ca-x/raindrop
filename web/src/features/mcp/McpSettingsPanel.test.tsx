import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, expect, it, vi } from "vitest"
import { Providers } from "../../app/Providers"
import { ApiClientError } from "../../shared/api/client"
import { activateLocale } from "../../shared/i18n/i18n"
import { McpSettingsPanel } from "./McpSettingsPanel"
import { createMcpToken, listMcpTokens, revokeMcpToken, type McpToken } from "./api"

vi.mock("./api", () => ({ listMcpTokens: vi.fn(), createMcpToken: vi.fn(), revokeMcpToken: vi.fn() }))
const token: McpToken = { id: "token-1", name: "Desktop AI", prefix: "rd_mcp_abc",
  createdAt: "2026-11-11T12:00:00Z", expiresAt: "2027-02-09T12:00:00Z", lastUsedAt: null }

beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(listMcpTokens).mockResolvedValue([])
  activateLocale("en")
})

function mount(onUnauthenticated?: () => void) {
  return render(<Providers><McpSettingsPanel csrfToken="csrf-test" onUnauthenticated={onUnauthenticated} /></Providers>)
}

it("creates a named expiring token once, shows its secret only until dismissed, and never includes it in config", async () => {
  vi.mocked(createMcpToken).mockResolvedValue({ item: token, token: "secret-only-once" })
  const user = userEvent.setup()
  const view = mount()
  await user.type(await screen.findByRole("textbox", { name: /^Token name/u }), "Desktop AI")
  await user.click(screen.getByRole("button", { name: "Create token" }))
  const secret = await screen.findByRole("textbox", { name: "Access token" })
  expect(secret).toHaveValue("secret-only-once")
  expect(secret).toHaveFocus()
  expect(createMcpToken).toHaveBeenCalledExactlyOnceWith("Desktop AI", 90, "csrf-test")
  await user.click(screen.getByText("Connect external AI"))
  expect(screen.getByLabelText("Example MCP client configuration")).not.toHaveTextContent("secret-only-once")
  await user.click(screen.getByRole("button", { name: "I saved it — hide token" }))
  expect(screen.queryByRole("textbox", { name: "Access token" })).not.toBeInTheDocument()
  expect(view.container.textContent).not.toContain("secret-only-once")
  expect(screen.getByText("Desktop AI")).toBeVisible()
  expect(screen.getByRole("button", { name: "Create token" })).toHaveFocus()
  expect(screen.queryByRole("textbox", { name: /^Token name/u })).not.toBeInTheDocument()
  await user.click(screen.getByRole("button", { name: "Create token" }))
  expect(screen.getByRole("textbox", { name: /^Token name/u })).toHaveFocus()
})

it("requires confirmation before revoking and keeps failed revocations retryable", async () => {
  vi.mocked(listMcpTokens).mockResolvedValue([token])
  vi.mocked(revokeMcpToken).mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(undefined)
  const user = userEvent.setup()
  mount()
  await user.click(await screen.findByRole("button", { name: "Revoke Desktop AI" }))
  expect(revokeMcpToken).not.toHaveBeenCalled()
  const dialog = screen.getByRole("alertdialog")
  await user.click(within(dialog).getByRole("button", { name: "Revoke" }))
  expect(await screen.findByRole("alert")).toBeVisible()
  expect(screen.getByRole("alertdialog")).toBeVisible()
  await user.click(within(dialog).getByRole("button", { name: "Revoke" }))
  await waitFor(() => expect(screen.queryByRole("button", { name: "Revoke Desktop AI" })).not.toBeInTheDocument())
  expect(revokeMcpToken).toHaveBeenLastCalledWith("token-1", "csrf-test")
  expect(screen.getByText("Token revoked. Clients can no longer use this credential.")).toBeVisible()
})

it("retries initial failures and handles expired sessions", async () => {
  const onUnauthenticated = vi.fn()
  vi.mocked(listMcpTokens).mockRejectedValueOnce(new ApiClientError(401, { code: "AUTHENTICATION_REQUIRED", message: "login" }))
    .mockResolvedValueOnce([])
  mount(onUnauthenticated)
  expect(await screen.findByRole("alert")).toBeVisible()
  expect(onUnauthenticated).toHaveBeenCalledOnce()
  await userEvent.click(screen.getByRole("button", { name: "Retry" }))
  expect(await screen.findByText(/No tokens yet/)).toBeVisible()
})

it("retains the one-time token and explains manual copying when clipboard fails", async () => {
  vi.mocked(createMcpToken).mockResolvedValue({ item: token, token: "secret-only-once" })
  const user = userEvent.setup()
  vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(new Error("denied"))
  mount()
  await user.type(await screen.findByRole("textbox", { name: /^Token name/u }), "Desktop AI")
  await user.click(screen.getByRole("button", { name: "Create token" }))
  await user.click(await screen.findByRole("button", { name: "Copy token" }))
  expect(await screen.findByText(/Clipboard unavailable/)).toBeVisible()
  expect(screen.getByRole("textbox", { name: "Access token" })).toHaveValue("secret-only-once")
})

it("rejects blank and overlong names without sending requests", async () => {
  const user = userEvent.setup()
  mount()
  const field = await screen.findByRole("textbox", { name: /^Token name/u })
  expect(screen.getByRole("button", { name: "Create token" })).toBeDisabled()
  await user.type(field, " ")
  expect(screen.getByRole("button", { name: "Create token" })).toBeDisabled()
  await user.type(field, "a".repeat(81))
  expect(screen.getByRole("button", { name: "Create token" })).toBeDisabled()
  expect(createMcpToken).not.toHaveBeenCalled()
})
