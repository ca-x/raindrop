import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { UserPreferences } from "../api/preferences.generated"
import { PreferencesDialog } from "./PreferencesDialog"

const preferences: UserPreferences = {
  locale: "en",
  themeMode: "SYSTEM",
  layoutDensity: "BALANCED",
  readingFontScale: 100,
}

it("edits all four values through ASTRYX controls and saves once", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onSave = vi.fn().mockResolvedValue(true)
  const onOpenChange = vi.fn()
  renderDialog({ onSave, onOpenChange })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  await user.click(within(dialog).getByRole("radio", { name: "Dark" }))
  await user.click(within(dialog).getByRole("radio", { name: "中文" }))
  await user.click(within(dialog).getByRole("radio", { name: "Compact" }))
  await user.click(within(dialog).getByRole("radio", { name: "120%" }))
  await user.click(within(dialog).getByRole("button", { name: "Save changes" }))

  expect(onSave).toHaveBeenCalledOnce()
  expect(onSave).toHaveBeenCalledWith({
    locale: "zh-CN",
    themeMode: "DARK",
    layoutDensity: "COMPACT",
    readingFontScale: 120,
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
        preferences={preferences}
        isSaving={false}
        error="SAVE"
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onSave={onSave}
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

it("keeps a save error while the draft is edited for a retry", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onClearError = vi.fn()
  renderDialog({ error: "SAVE", onClearError })
  const dialog = screen.getByRole("dialog", { name: "Settings" })

  await user.click(within(dialog).getByRole("radio", { name: "Dark" }))

  expect(within(dialog).getByText("Preferences could not be saved")).toBeVisible()
  expect(onClearError).not.toHaveBeenCalled()
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
  expect(within(dialog).getByText("调整 Raindrop 的显示方式，不打断当前阅读。")).toBeVisible()
  expect(within(dialog).getByRole("radio", { name: "跟随系统" })).toBeVisible()
  expect(within(dialog).getByRole("radio", { name: "均衡" })).toBeVisible()
  expect(within(dialog).getByRole("button", { name: "保存更改" })).toBeVisible()
})

function renderDialog(
  overrides: Partial<React.ComponentProps<typeof PreferencesDialog>> = {},
) {
  const props: React.ComponentProps<typeof PreferencesDialog> = {
    isOpen: true,
    preferences,
    isSaving: false,
    error: null,
    onOpenChange: vi.fn(),
    onClearError: vi.fn(),
    onSave: vi.fn().mockResolvedValue(true),
    ...overrides,
  }
  return render(
    <Providers>
      <PreferencesDialog {...props} />
    </Providers>,
  )
}
