import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { Link, Route, Routes } from "react-router-dom"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { TranslationConfig } from "../api/translation.generated"
import type { EntryTranslationController } from "../model/useEntryTranslationController"
import { TranslationReaderControls } from "./TranslationReaderControls"

it("keeps router navigation responsive while article translation is pending", async () => {
  activateLocale("en")
  window.history.replaceState(null, "", "/reader")
  const user = userEvent.setup()
  const pending = deferred<boolean>()
  const controller = createController({
    translate: vi.fn(() => pending.promise),
  })

  render(
    <Providers>
      <Routes>
        <Route
          path="/reader"
          element={
            <>
              <TranslationReaderControls
                controller={controller}
                config={config}
                onDisplayModeChange={vi.fn().mockResolvedValue(true)}
              />
              <Link to="/settings">Settings</Link>
            </>
          }
        />
        <Route path="/settings" element={<h1>Settings page</h1>} />
      </Routes>
    </Providers>,
  )

  await user.click(
    screen.getByRole("button", { name: "Translate article" }),
  )
  await user.click(screen.getByRole("link", { name: "Settings" }))

  expect(screen.getByRole("heading", { name: "Settings page" })).toBeVisible()
  expect(controller.translate).toHaveBeenCalledTimes(1)
})

it("shows only article-level failures in the translation toolbar", () => {
  activateLocale("en")
  const { rerender } = render(
    <Providers>
      <TranslationReaderControls
        controller={createController({
          contextError: "RATE_LIMITED",
        })}
        config={config}
        onDisplayModeChange={vi.fn().mockResolvedValue(true)}
      />
    </Providers>,
  )
  expect(
    screen.queryByText("Too many requests. Try again later."),
  ).not.toBeInTheDocument()

  rerender(
    <Providers>
      <TranslationReaderControls
        controller={createController({
          articleError: "RATE_LIMITED",
        })}
        config={config}
        onDisplayModeChange={vi.fn().mockResolvedValue(true)}
      />
    </Providers>,
  )
  expect(
    screen.getByText("Too many requests. Try again later."),
  ).toBeInTheDocument()
})

function createController(
  overrides: Partial<EntryTranslationController> = {},
): EntryTranslationController {
  return {
    entryId: "00000000-0000-4000-8000-000000000301",
    result: null,
    lookupResult: null,
    selectionResult: null,
    isTranslating: false,
    isLookingUp: false,
    isTranslatingSelection: false,
    completedSegments: 0,
    totalSegments: 0,
    articleError: null,
    contextError: null,
    translate: vi.fn().mockResolvedValue(true),
    lookup: vi.fn().mockResolvedValue(true),
    translateSelection: vi.fn().mockResolvedValue(true),
    clearTranslation: vi.fn(),
    clearLookup: vi.fn(),
    clearSelectionTranslation: vi.fn(),
    cancelContextActions: vi.fn(),
    clearError: vi.fn(),
    ...overrides,
  }
}

const config: TranslationConfig = {
  engine: "DEEPLX",
  displayMode: "BILINGUAL",
  isEnabled: true,
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
  revision: 1,
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}
