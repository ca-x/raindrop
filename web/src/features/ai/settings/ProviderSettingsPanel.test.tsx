import { act, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { fakeAiSettingsController } from "../model/testFixtures"
import { ProviderSettingsPanel } from "./ProviderSettingsPanel"

afterEach(() => vi.unstubAllGlobals())

function renderPanel(controller = fakeAiSettingsController()) {
  activateLocale("en")
  render(<Providers><ProviderSettingsPanel controller={controller} /></Providers>)
  return controller
}

it("lets an existing provider change API type while preserving its identity and credential", async () => {
  const user = userEvent.setup()
  const controller = renderPanel()
  await user.click(screen.getByRole("button", { name: "Edit" }))
  await user.click(screen.getByRole("combobox", { name: "Provider kind" }))
  await user.keyboard("{ArrowUp}{Enter}")
  await user.click(screen.getByRole("button", { name: "Save Provider" }))
  expect(controller.saveProvider).toHaveBeenCalledWith(expect.objectContaining({
    providerId: controller.providers[0]!.providerId, kind: "ANTHROPIC_MESSAGES", credential: "",
    endpoint: "https://api.anthropic.com/",
  }))
  expect(screen.getByText("Provider saved.")).toBeVisible()
})

it("discovers models for a disabled saved provider without entering its secret", async () => {
  const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ models: [{ id: "model-new" }] })))
  vi.stubGlobal("fetch", fetchMock)
  const user = userEvent.setup()
  const controller = fakeAiSettingsController()
  controller.providers[0]!.isEnabled = false
  renderPanel(controller)
  await user.click(screen.getByRole("button", { name: "Edit" }))
  await user.click(screen.getByRole("button", { name: "Fetch models" }))
  await waitFor(() => expect(fetchMock).toHaveBeenCalledOnce())
  expect(JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body))).toMatchObject({
    providerId: controller.providers[0]!.providerId, credential: "",
  })
  expect(await screen.findByText("1 available models loaded.")).toBeInTheDocument()
})

it("cancels model discovery when the endpoint changes and ignores stale results", async () => {
  let resolve!: (response: Response) => void
  const fetchMock = vi.fn(() => new Promise<Response>((done) => { resolve = done }))
  vi.stubGlobal("fetch", fetchMock)
  const user = userEvent.setup()
  renderPanel()
  await user.click(screen.getByRole("button", { name: "Edit" }))
  await user.click(screen.getByRole("button", { name: "Fetch models" }))
  const signal = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].signal!
  await user.clear(screen.getByRole("textbox", { name: /^Endpoint/u }))
  expect(signal.aborted).toBe(true)
  await act(async () => resolve(new Response(JSON.stringify({ models: [{ id: "stale-model" }] }))))
  expect(screen.queryByText(/available models loaded/u)).not.toBeInTheDocument()
  expect(screen.getByRole("button", { name: "Fetch models" })).toBeEnabled()
})

it("shows actionable authentication failures and leaves manual model entry available", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({
    error: { code: "MODEL_DISCOVERY_AUTH_FAILED", message: "Upstream failed" },
  }), { status: 502 })))
  const user = userEvent.setup()
  renderPanel()
  await user.click(screen.getByRole("button", { name: "Edit" }))
  await user.click(screen.getByRole("button", { name: "Fetch models" }))
  expect(await screen.findByText(/The service rejected the credential/u)).toBeVisible()
  expect(screen.getByRole("textbox", { name: /^Model/u })).toBeEnabled()
  expect(screen.getByRole("button", { name: "Save Provider" })).toBeEnabled()
})

it("requires confirmation to delete and lets the user cancel without a mutation", async () => {
  const user = userEvent.setup()
  const controller = renderPanel()
  await user.click(screen.getByRole("button", { name: "Delete" }))
  let dialog = screen.getByRole("alertdialog")
  expect(dialog).toHaveTextContent("Primary model")
  expect(dialog).toHaveTextContent("Existing articles and generated results will be kept")
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
  expect(controller.removeProvider).not.toHaveBeenCalled()
  await user.click(screen.getByRole("button", { name: "Delete" }))
  dialog = screen.getByRole("alertdialog")
  await user.click(within(dialog).getByRole("button", { name: "Delete" }))
  expect(controller.removeProvider).toHaveBeenCalledWith(controller.providers[0])
  expect(await screen.findByText("Provider deleted.")).toBeVisible()
})
