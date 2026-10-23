import { render, screen } from "@testing-library/react"
import { expect, it } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { ReadingHighlights, ReadingQuestions } from "./ReadingCards"

it("bounds streamed content and treats model HTML as text", () => {
  const { container } = render(<ReadingHighlights title={'<script>alert("xss")</script>'}
    points={Array.from({ length: 10 }, () => "<img onerror=alert(1)>" + "a".repeat(3000))} />)
  expect(screen.getAllByRole("listitem")).toHaveLength(6)
  expect(screen.getAllByRole("listitem")[0].textContent).toHaveLength(2000)
  expect(container.querySelector("script, img")).toBeNull()
})

it("keeps answers collapsed and tolerates incomplete streaming props", () => {
  activateLocale("en")
  const { container, rerender } = render(<Providers><ReadingQuestions /></Providers>)
  expect(container.querySelector("details")).toBeNull()
  rerender(<Providers><ReadingQuestions questions={[{ question: "Why?", answer: "Because." }]} /></Providers>)
  expect(container.querySelector("details")).not.toHaveAttribute("open")
  expect(screen.getByText("Because.")).not.toBeVisible()
})
