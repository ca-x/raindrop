import { render, screen } from "@testing-library/react"
import { expect, it } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import {
  safeExternalMarkdownLink,
  TranslationView,
} from "./TranslationView"

it("renders validated Markdown without using article HTML", () => {
  activateLocale("en")
  render(
    <Providers>
      <TranslationView
        artifact={{
          artifactId: "00000000-0000-4000-8000-000000000502",
          kind: "AI_TRANSLATION",
          providerLabel: "Primary model",
          createdAt: "2026-07-20T10:00:00Z",
          detectedSourceLanguage: "en",
          targetLocale: "zh-CN",
          title: "译文标题",
          bodyMarkdown: "## 小节\n\n[安全链接](https://example.com/)\n\n<script>blocked</script>",
        }}
      />
    </Providers>,
  )

  expect(screen.getByRole("heading", { name: "译文标题", level: 3 })).toBeVisible()
  expect(screen.getByRole("heading", { name: "小节", level: 4 })).toBeVisible()
  expect(screen.getByRole("link", { name: "安全链接" })).toHaveAttribute(
    "href",
    "https://example.com/",
  )
  expect(document.querySelector("script")).not.toBeInTheDocument()
  expect(screen.getByText(/blocked/u)).toBeVisible()
})

it("permits only absolute HTTP and HTTPS Markdown links", () => {
  const event = {} as React.MouseEvent<HTMLAnchorElement>
  expect(safeExternalMarkdownLink("https://example.com", event)).toBeUndefined()
  expect(safeExternalMarkdownLink("http://example.com", event)).toBeUndefined()
  expect(safeExternalMarkdownLink("javascript:alert(1)", event)).toBe(false)
  expect(safeExternalMarkdownLink("data:text/html,unsafe", event)).toBe(false)
  expect(safeExternalMarkdownLink("/relative", event)).toBe(false)
})
