import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { Provider } from "../../ai/api/provider.generated"
import type { TranslationConfig } from "../api/translation.generated"
import type { TranslationSettingsController } from "../model/useTranslationSettingsController"
import { TranslationSettingsPanel } from "./TranslationSettingsPanel"

it("fills a safe custom prompt when the custom expert is selected", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  renderPanel({
    config: { ...baseConfig, engine: "OPENAI" },
    providers: [provider],
  })

  const profile = await screen.findByRole("combobox", { name: "Translation expert" })
  await user.click(profile)
  await user.keyboard("{End}{Enter}")

  expect(
    screen.getByText(/Starts with a safe general template/u),
  ).toBeVisible()
  expect(
    screen.getByRole<HTMLTextAreaElement>("textbox", { name: /^System prompt/u })
      .value,
  ).toContain("{{to}}")
  expect(
    screen.getByRole<HTMLTextAreaElement>("textbox", {
      name: /^Translation prompt/u,
    }).value,
  ).toContain("{{text}}")

})

it("shows the selected translation expert's purpose", async () => {
  activateLocale("en")
  renderPanel({
    config: {
      ...baseConfig,
      engine: "OPENAI",
      openAi: { ...baseConfig.openAi, profile: "TECHNICAL" },
    },
    providers: [provider],
  })

  expect(
    await screen.findByText(
      /Preserves code, commands, paths, APIs, and established English terms/u,
    ),
  ).toBeVisible()
  expect(screen.queryByRole("textbox", { name: /^System prompt/u })).not.toBeInTheDocument()
})

it("keeps the DeepLX API key optional and explicitly removes a saved key", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const save = vi.fn().mockResolvedValue(true)
  renderPanel({
    config: {
      ...baseConfig,
      deepLx: { ...baseConfig.deepLx, hasApiKey: true },
      revision: 4,
    },
    save,
  })

  expect(
    await screen.findByText("A key is saved. Leave this blank to keep it."),
  ).toBeVisible()
  await user.click(screen.getByRole("button", { name: "Remove saved API Key" }))
  expect(screen.getByText("API Key will be removed")).toBeVisible()
  await user.click(
    screen.getByRole("button", { name: "Save translation settings" }),
  )

  expect(save).toHaveBeenCalledWith(
    expect.objectContaining({
      expectedRevision: 4,
      engine: "DEEPLX",
      deepLx: expect.objectContaining({ apiKey: null }),
    }),
  )
})

it("saves the DeepLX progressive article preference", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const save = vi.fn().mockResolvedValue(true)
  renderPanel({ save })

  const progressive = await screen.findByRole("switch", {
    name: "Progressive article translation",
  })
  expect(progressive).toBeChecked()
  await user.click(progressive)
  await user.click(
    screen.getByRole("button", { name: "Save translation settings" }),
  )

  expect(save).toHaveBeenCalledWith(
    expect.objectContaining({
      deepLx: expect.objectContaining({ isProgressive: false }),
    }),
  )
})

function renderPanel({
  config = baseConfig,
  providers = [],
  save = vi.fn().mockResolvedValue(true),
}: {
  config?: TranslationConfig
  providers?: Provider[]
  save?: TranslationSettingsController["save"]
} = {}) {
  const controller: TranslationSettingsController = {
    config,
    loadStatus: "ready",
    error: null,
    isSaving: false,
    isTesting: false,
    testResult: null,
    load: vi.fn().mockResolvedValue(undefined),
    save,
    saveDisplayMode: vi.fn().mockResolvedValue(true),
    test: vi.fn().mockResolvedValue(true),
    clearError: vi.fn(),
    cancel: vi.fn(),
  }
  return render(
    <Providers>
      <TranslationSettingsPanel controller={controller} providers={providers} />
    </Providers>,
  )
}

const provider: Provider = {
  providerId: "00000000-0000-4000-8000-000000000101",
  scope: "USER",
  canEdit: true,
  displayName: "OpenAI compatible",
  kind: "OPENAI_RESPONSES",
  endpoint: "https://api.openai.com/v1/responses",
  model: "gpt-5-mini",
  capabilities: { supportsUsage: true, supportsIdempotency: true },
  policy: {
    maxConcurrency: 4,
    requestsPerMinute: 60,
    maxInputTokensPerRequest: 32_768,
    maxOutputTokensPerRequest: 16_384,
    inputCostMicrosPerMillionTokens: null,
    outputCostMicrosPerMillionTokens: null,
    maxCostMicrosPerRequest: null,
  },
  isEnabled: true,
  revision: 1,
  createdAt: "2026-07-21T00:00:00Z",
  updatedAt: "2026-07-21T00:00:00Z",
}

const baseConfig: TranslationConfig = {
  engine: "DEEPLX",
  displayMode: "BILINGUAL",
  isEnabled: true,
  defaultTargetLocale: "zh-CN",
  openAi: {
    providerId: provider.providerId,
    maxOutputTokens: 4096,
    profile: "GENERAL",
    customSystemPrompt: null,
    customPrompt: null,
  },
  deepLx: {
    displayName: "DeepLX",
    description: null,
    baseUrl: null,
    isProgressive: true,
    hasApiKey: false,
  },
  revision: null,
}
