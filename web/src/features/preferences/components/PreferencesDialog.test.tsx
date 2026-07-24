import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { fakeAiSettingsController } from "../../ai/model/testFixtures"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import type { UserPreferences } from "../api/preferences.generated"
import { PreferencesDialog } from "./PreferencesDialog"

const preferences: UserPreferences = {
  locale: "en",
  themeMode: "SYSTEM",
  layoutDensity: "BALANCED",
  readingFontScale: 100,
  readingFontFamily: "SERIF",
  readingCustomFontId: null,
  readingColorScheme: "AUTO",
  linkOpenMode: "NEW_TAB",
}

it("edits personal and reading preferences through ASTRYX controls and saves once", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onSave = vi.fn().mockResolvedValue(true)
  const onOpenChange = vi.fn()
  renderDialog({ onSave, onOpenChange })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  await user.click(within(dialog).getByRole("radio", { name: "Dark" }))
  await user.click(within(dialog).getByRole("radio", { name: "中文" }))
  await user.click(within(dialog).getByRole("radio", { name: "Compact" }))
  await user.click(within(dialog).getByRole("button", { name: /^Reading/ }))
  expect(within(dialog).queryByRole("combobox", { name: "Article font" })).not.toBeInTheDocument()
  expect(within(dialog).queryByRole("slider", { name: "Reading size" })).not.toBeInTheDocument()
  await user.click(within(dialog).getByRole("radio", { name: "Sepia" }))
  await user.click(within(dialog).getByRole("radio", { name: "Current page" }))
  await user.click(within(dialog).getByRole("button", { name: "Save changes" }))

  expect(onSave).toHaveBeenCalledOnce()
  expect(onSave).toHaveBeenCalledWith({
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: "COMPACT",
    readingFontScale: 100,
    readingFontFamily: "SERIF",
    readingCustomFontId: null,
    readingColorScheme: "SEPIA",
    linkOpenMode: "CURRENT_TAB",
  })
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
})

it("preserves the draft and inline error when saving fails", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onSave = vi.fn().mockResolvedValue(false)
  const { rerender } = renderDialog({ onSave })
  let dialog = screen.getByRole("dialog", { name: "Settings" })
  await user.click(within(dialog).getByRole("radio", { name: "Dark" }))
  await user.click(within(dialog).getByRole("button", { name: "Save changes" }))

  rerender(
    <Providers>
      <PreferencesDialog
        isOpen
        profile={{
          userId: "00000000-0000-4000-8000-000000000001",
          username: "reader",
          displayName: null,
          email: null,
        }}
        preferences={preferences}
        fonts={[]}
        fontLimits={{ maximumCount: 8, maximumBytes: 5 * 1024 * 1024 }}
        isSaving={false}
        isProfileSaving={false}
        isFontMutating={false}
        error="SAVE"
        profileError={null}
        profileFieldErrors={{}}
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onClearProfileError={vi.fn()}
        onSave={onSave}
        onSaveProfile={vi.fn().mockResolvedValue(true)}
        onUploadFont={vi.fn().mockResolvedValue(true)}
        onDeleteFont={vi.fn().mockResolvedValue(true)}
      />
    </Providers>,
  )
  dialog = screen.getByRole("dialog", { name: "Settings" })
  expect(within(dialog).getByText("Preferences could not be saved")).toBeVisible()
  expect(within(dialog).getByRole("radio", { name: "Dark" })).toHaveAttribute(
    "aria-checked",
    "true",
  )
})

it("cancels without saving and uses a viewport-bounded form dialog", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onSave = vi.fn()
  const onOpenChange = vi.fn()
  renderDialog({ onSave, onOpenChange })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  expect(dialog).toHaveClass("reader-preferences-dialog")
  expect(dialog.getAttribute("style")).toContain("100vw - 24px")
  expect(dialog.getAttribute("style")).toContain("100dvh - 24px")
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }))

  expect(onSave).not.toHaveBeenCalled()
  expect(onOpenChange).toHaveBeenCalledWith(false)
})

it("renders the complete settings workflow in Chinese", () => {
  activateLocale("zh-CN")
  renderDialog()

  const dialog = screen.getByRole("dialog", { name: "设置" })
  expect(within(dialog).getByText("管理账户、阅读、插件与备份设置。")).toBeVisible()
  expect(within(dialog).getByRole("radio", { name: "跟随系统" })).toBeVisible()
  expect(within(dialog).getByRole("radio", { name: "均衡" })).toBeVisible()
  expect(within(dialog).getByRole("button", { name: "保存更改" })).toBeVisible()
})

it("uses functional settings descriptions in English", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  renderDialog({
    aiController: fakeAiSettingsController(),
    translationController: fakeTranslationSettingsController(),
  })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  expect(within(dialog).getByText("Manage account, reading, plugins, and backups.")).toBeVisible()
  expect(within(dialog).getByText("Nickname, email, theme, language, and interface density.")).toBeVisible()
  await user.click(within(dialog).getByRole("button", { name: /^Plugins/ }))
  expect(within(dialog).getByText("AI Providers, article summaries, and translation.")).toBeVisible()
  expect(
    within(dialog).getByText(
      "Translate full articles and look up words with OpenAI or DeepLX.",
    ),
  ).toBeVisible()
})

it("keeps subscription transfer out of settings", () => {
  activateLocale("en")
  renderDialog()
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  expect(within(dialog).queryByRole("button", { name: "Subscriptions" })).not.toBeInTheDocument()
  expect(within(dialog).queryByLabelText("OPML file")).not.toBeInTheDocument()
})

it("uses icon-assisted navigation and exposes the current build version", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  renderDialog()
  const dialog = screen.getByRole("dialog", { name: "Settings" })
  const navigation = within(dialog).getByRole("navigation", { name: "Settings sections" })

  expect(navigation.querySelectorAll("svg").length).toBeGreaterThanOrEqual(3)
  await user.click(within(navigation).getByRole("button", { name: /^About/ }))
  expect(within(dialog).getByText("Raindrop")).toBeVisible()
  expect(within(dialog).getByText("v0.4.3")).toBeVisible()
})

it("clears a deleted active custom font from the open draft before saving", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const fontId = "00000000-0000-4000-8000-000000000701"
  const onSave = vi.fn().mockResolvedValue(true)
  const onDeleteFont = vi.fn().mockResolvedValue(true)
  renderDialog({
    initialTab: "reading",
    preferences: { ...preferences, readingCustomFontId: fontId },
    fonts: [{
      fontId,
      displayName: "Reader Serif",
      byteSize: 24_000,
      fileUrl: `/api/v2/preferences/fonts/${fontId}/file`,
    }],
    onSave,
    onDeleteFont,
  })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  await user.click(
    within(dialog).getByRole("button", { name: "Delete “Reader Serif”" }),
  )
  expect(
    within(dialog).getByRole("button", { name: "Delete “Reader Serif”" }),
  ).not.toHaveTextContent("Reader Serif")
  await user.click(within(dialog).getByRole("button", { name: "Save changes" }))

  expect(onDeleteFont).toHaveBeenCalledWith(fontId)
  expect(onSave).toHaveBeenCalledWith({
    ...preferences,
    readingCustomFontId: null,
  })
})

it("keeps plugin saves separate from the preference controller", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onSave = vi.fn().mockResolvedValue(true)
  const saveConfig = vi.fn().mockResolvedValue(true)
  renderDialog({
    onSave,
    aiController: fakeAiSettingsController({ saveConfig }),
    translationController: fakeTranslationSettingsController(),
  })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  await user.click(within(dialog).getByRole("button", { name: /^Plugins/ }))
  expect(
    within(dialog).getByRole("button", { name: "Settings for AI Provider" }),
  ).toBeVisible()
  expect(
    within(dialog).getByRole("button", { name: "Settings for Translation" }),
  ).toBeVisible()
  const assistant = within(dialog).getByText("AI Assistant").closest("li")
  expect(assistant).not.toBeNull()
  await user.click(
    within(assistant as HTMLElement).getByRole("button", {
      name: "Settings for AI Assistant",
    }),
  )
  await user.click(
    within(dialog).getByRole("switch", { name: "Enable AI reading plugin" }),
  )
  await user.click(
    within(dialog).getByRole("button", { name: "Save AI settings" }),
  )

  expect(saveConfig).toHaveBeenCalledOnce()
  expect(onSave).not.toHaveBeenCalled()
  expect(within(dialog).getByRole("button", { name: "Close" })).toBeVisible()
})

function renderDialog(
  overrides: Partial<React.ComponentProps<typeof PreferencesDialog>> = {},
) {
  const props: React.ComponentProps<typeof PreferencesDialog> = {
    isOpen: true,
    profile: {
      userId: "00000000-0000-4000-8000-000000000001",
      username: "reader",
      displayName: null,
      email: null,
    },
    preferences,
    fonts: [],
    fontLimits: { maximumCount: 8, maximumBytes: 5 * 1024 * 1024 },
    isSaving: false,
    isProfileSaving: false,
    isFontMutating: false,
    error: null,
    profileError: null,
    profileFieldErrors: {},
    onOpenChange: vi.fn(),
    onClearError: vi.fn(),
    onClearProfileError: vi.fn(),
    onSave: vi.fn().mockResolvedValue(true),
    onSaveProfile: vi.fn().mockResolvedValue(true),
    onUploadFont: vi.fn().mockResolvedValue(true),
    onDeleteFont: vi.fn().mockResolvedValue(true),
    ...overrides,
  }
  return render(
    <Providers>
      <PreferencesDialog {...props} />
    </Providers>,
  )
}

function fakeTranslationSettingsController(): TranslationSettingsController {
  return {
    config: {
      engine: "DEEPLX",
      displayMode: "BILINGUAL",
      isEnabled: false,
      defaultTargetLocale: "zh-CN",
      openAi: {
        providerId: null,
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
    },
    loadStatus: "ready",
    error: null,
    isSaving: false,
    isTesting: false,
    testResult: null,
    load: vi.fn().mockResolvedValue(undefined),
    save: vi.fn().mockResolvedValue(true),
    saveDisplayMode: vi.fn().mockResolvedValue(true),
    test: vi.fn().mockResolvedValue(true),
    clearError: vi.fn(),
    cancel: vi.fn(),
  }
}
