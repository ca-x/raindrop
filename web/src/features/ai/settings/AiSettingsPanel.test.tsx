import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { fakeAiSettingsController } from "../model/testFixtures"
import { AiSettingsPanel } from "./AiSettingsPanel"

it("creates a Provider inline and never renders a credential readback", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const saveProvider = vi.fn().mockResolvedValue(false)
  renderPanel(fakeAiSettingsController({ saveProvider }))

  await user.click(screen.getByRole("button", { name: "Add Provider" }))
  const form = screen.getByRole("button", { name: "New Provider" }).closest("div")!
  await user.type(screen.getByRole("textbox", { name: /^Display name/u }), "Local model")
  await user.type(screen.getByRole("textbox", { name: /^Model/u }), "model-v1")
  await user.type(
    screen.getByLabelText(/^Credential/u),
    "credential-kept-in-memory-only",
  )
  await user.click(screen.getByRole("button", { name: "Create Provider" }))

  expect(saveProvider).toHaveBeenCalledWith(
    expect.objectContaining({
      displayName: "Local model",
      model: "model-v1",
      credential: "credential-kept-in-memory-only",
    }),
  )
  expect(screen.getByLabelText(/^Credential/u)).toHaveValue(
    "credential-kept-in-memory-only",
  )
  expect(screen.queryByText(/credential-kept-in-memory-only/u)).not.toBeInTheDocument()
})

it("edits only user Providers and leaves the credential field empty", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const controller = fakeAiSettingsController()
  const instance = {
    ...controller.providers[0]!,
    providerId: "00000000-0000-4000-8000-000000000102",
    scope: "INSTANCE" as const,
    canEdit: false,
    displayName: "Instance model",
  }
  renderPanel(
    fakeAiSettingsController({ providers: [instance, controller.providers[0]!] }),
  )

  expect(screen.getAllByRole("button", { name: "Edit" })).toHaveLength(1)
  await user.click(screen.getByRole("button", { name: "Edit" }))

  expect(screen.getByLabelText(/^Credential/u)).toHaveValue("")
  expect(
    screen.getByText("Leave blank to keep the existing credential."),
  ).toBeVisible()
  expect(screen.getByRole("combobox", { name: "Provider kind" })).toHaveAttribute(
    "aria-disabled",
    "true",
  )
})

it("saves independent AI content config with the selected Provider", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const saveConfig = vi.fn().mockResolvedValue(true)
  renderPanel(fakeAiSettingsController({ saveConfig }))

  await user.click(
    screen.getByRole("switch", { name: "Enable AI reading plugin" }),
  )
  await user.click(screen.getByRole("button", { name: "Save AI settings" }))

  expect(saveConfig).toHaveBeenCalledWith({
    expectedRevision: null,
    isEnabled: true,
    summary: {
      enabled: true,
      providerId: "00000000-0000-4000-8000-000000000101",
      style: "BALANCED",
      maxOutputTokens: 1024,
    },
    translation: {
      enabled: false,
      providerId: "00000000-0000-4000-8000-000000000101",
      defaultTargetLocale: "zh-CN",
      maxOutputTokens: 4096,
    },
  })
})

it("allows an enabled AI plugin to be disabled after all Providers are disabled", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const controller = fakeAiSettingsController()
  const provider = { ...controller.providers[0]!, isEnabled: false }
  const saveConfig = vi.fn().mockResolvedValue(true)
  renderPanel(
    fakeAiSettingsController({
      providers: [provider],
      saveConfig,
      configEnvelope: {
        pluginState: "READY",
        mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
        config: {
          revision: 3,
          isEnabled: true,
          summary: {
            enabled: true,
            providerId: provider.providerId,
            style: "BALANCED",
            maxOutputTokens: 1024,
          },
          translation: {
            enabled: false,
            providerId: provider.providerId,
            defaultTargetLocale: "zh-CN",
            maxOutputTokens: 4096,
          },
        },
      },
    }),
  )

  await user.click(
    screen.getByRole("switch", { name: "Enable AI reading plugin" }),
  )
  await user.click(screen.getByRole("button", { name: "Save AI settings" }))

  expect(saveConfig).toHaveBeenCalledWith(
    expect.objectContaining({
      expectedRevision: 3,
      isEnabled: false,
      summary: expect.objectContaining({ enabled: true }),
      translation: expect.objectContaining({ enabled: false }),
    }),
  )
})

it("shows explicit keyring, plugin, and MCP availability without fake controls", () => {
  activateLocale("en")
  renderPanel(
    fakeAiSettingsController({
      keyringStatus: "UNAVAILABLE",
      configEnvelope: {
        pluginState: "QUARANTINED",
        mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
        config: null,
      },
    }),
  )

  expect(screen.getByText("Provider keyring unavailable")).toBeVisible()
  expect(screen.getByText("Official AI plugin unavailable")).toBeVisible()
  expect(screen.getByText("MCP transport is not available yet")).toBeVisible()
  expect(screen.queryByRole("checkbox", { name: /MCP/iu })).not.toBeInTheDocument()
  expect(screen.getByRole("button", { name: "Save AI settings" })).toBeDisabled()
})

function renderPanel(controller = fakeAiSettingsController()) {
  return render(
    <Providers>
      <AiSettingsPanel controller={controller} />
    </Providers>,
  )
}
