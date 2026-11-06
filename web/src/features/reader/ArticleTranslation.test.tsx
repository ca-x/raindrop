import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import type { TranslationConfig } from "../translation/api/translation.generated"
import { ArticleReader } from "./components/ArticleReader"
import { initialReaderState } from "./model/reducer"
import { makeDetail } from "./model/testFixtures"
import "./reader.css"

afterEach(() => vi.unstubAllGlobals())

it.each([
  ["plain text", "Hello world", ["Hello world"]],
  ["inline markup", "Hello <em>world</em>", ["Hello world"]],
  ["loose text around a paragraph", "Intro<p>Middle</p>Tail", ["Intro", "Middle", "Tail"]],
  ["mixed container", "<div>Intro<p>Middle</p>Tail</div>", ["Intro", "Middle", "Tail"]],
  ["definition and address blocks", "<dl><dt>Term</dt><dd>Definition</dd></dl><address>Contact</address>", ["Term", "Definition", "Contact"]],
] as const)("shows and clears translated %s without losing the source markup", async (_, contentHtml, originals) => {
  activateLocale("en")
  const detail = makeDetail({ contentHtml })
  const state = structuredClone(initialReaderState)
  state.selectedEntryId = detail.entryId
  state.detailsById[detail.entryId] = detail
  state.paneStatus.detail = "ready"
  vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({
    title: "Translated title",
    segments: originals.map((originalText, index) => ({ index, originalText, translatedText: `译文${index}` })),
    providerLabel: "Test provider",
    detectedSourceLocale: "en",
    targetLocale: "zh-CN",
  }), { headers: { "content-type": "application/json" } })))
  render(
    <Providers>
      <ArticleReader
        state={state}
        entryRoute={`/reader/unread/entry/${detail.entryId}`}
        routeEntryId={detail.entryId}
        savedScrollOffset={0}
        shouldFocusArticle={false}
        onRecordScroll={vi.fn()}
        onToggleRead={vi.fn().mockResolvedValue(undefined)}
        onToggleStar={vi.fn().mockResolvedValue(undefined)}
        csrfToken="csrf-test"
        translationConfig={config}
        translationSettingsController={{
          config, loadStatus: "ready", error: null, isSaving: false, isTesting: false, testResult: null,
          load: vi.fn().mockResolvedValue(undefined), save: vi.fn().mockResolvedValue(true),
          saveDisplayMode: vi.fn().mockResolvedValue(true), test: vi.fn().mockResolvedValue(true),
          clearError: vi.fn(), cancel: vi.fn(),
        }}
      />
    </Providers>,
  )
  const body = document.querySelector<HTMLElement>(".reader-article-body")!
  const originalHtml = body.innerHTML
  const user = userEvent.setup()
  await user.click(screen.getByRole("button", { name: "Translate article" }))
  for (let index = 0; index < originals.length; index += 1) {
    expect(await screen.findByText(`译文${index}`)).toBeVisible()
  }
  await user.click(screen.getByRole("button", { name: "Clear translation" }))
  await waitFor(() => expect(body.innerHTML).toBe(originalHtml))
  expect(body.querySelector(".reader-translation-segment")).toBeNull()
})

const config: TranslationConfig = {
  engine: "OPENAI", displayMode: "BILINGUAL", isEnabled: true, defaultTargetLocale: "zh-CN", revision: 1,
  openAi: { providerId: "00000000-0000-4000-8000-000000000901", maxOutputTokens: 4096, profile: "GENERAL", customSystemPrompt: null, customPrompt: null },
  deepLx: { displayName: "DeepLX", description: null, baseUrl: null, isProgressive: false, hasApiKey: false },
}
