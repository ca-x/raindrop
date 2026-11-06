import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { ApiClientError } from "../../../shared/api/client"
import { activateLocale } from "../../../shared/i18n/i18n"
import { askArticleQuestion } from "../api/questions"
import { ArticleQuestions } from "./ArticleQuestions"

vi.mock("../api/questions", () => ({ askArticleQuestion: vi.fn() }))
const props = { entryId: "entry-a", summaryJobId: "summary-a", csrfToken: "csrf" }
const answer = { answerMarkdown: "**Codex** helps teams build tools.", providerLabel: "Test model" }

beforeEach(() => { activateLocale("en"); vi.mocked(askArticleQuestion).mockReset() })

function view(overrides: Partial<typeof props> = {}) {
  return <Providers><ArticleQuestions {...props} {...overrides} /></Providers>
}

it("sends a question with context and includes the previous answer on a follow-up", async () => {
  const user = userEvent.setup()
  vi.mocked(askArticleQuestion).mockResolvedValue(answer)
  render(view())
  const input = screen.getByRole("textbox", { name: "Your question" })
  await user.type(input, "What is Codex?")
  await user.click(screen.getByRole("button", { name: "Send" }))
  await screen.findByText("helps teams build tools.", { exact: false })
  expect(askArticleQuestion).toHaveBeenCalledWith("entry-a", "csrf", {
    summaryJobId: "summary-a", question: "What is Codex?", responseLocale: "en", history: [],
  }, expect.any(AbortSignal))
  await user.type(input, "Give an example")
  await user.click(screen.getByRole("button", { name: "Send" }))
  await waitFor(() => expect(askArticleQuestion).toHaveBeenCalledTimes(2))
  expect(vi.mocked(askArticleQuestion).mock.calls[1][2].history).toEqual([{ question: "What is Codex?", answer: answer.answerMarkdown }])
})

it("keeps failed input, retries it and exposes the error beside the composer", async () => {
  const user = userEvent.setup()
  vi.mocked(askArticleQuestion).mockRejectedValueOnce(new ApiClientError(504, { code: "AI_QUESTION_TIMEOUT", message: "timeout" })).mockResolvedValue(answer)
  render(view())
  const input = screen.getByRole("textbox", { name: "Your question" })
  await user.type(input, "Explain the evidence")
  await user.click(screen.getByRole("button", { name: "Send" }))
  await screen.findByText("The model timed out. You can resend this question.")
  expect(input).toHaveValue("Explain the evidence")
  await user.click(screen.getByRole("button", { name: "Retry" }))
  await waitFor(() => expect(askArticleQuestion).toHaveBeenCalledTimes(2))
  await screen.findByText("helps teams build tools.", { exact: false })
})

it("stops a pending request and ignores a late response after the article changes", async () => {
  const user = userEvent.setup()
  let resolve!: (value: typeof answer) => void
  vi.mocked(askArticleQuestion).mockImplementation(() => new Promise((done) => { resolve = done }))
  const rendered = render(view())
  await user.type(screen.getByRole("textbox", { name: "Your question" }), "Explain this")
  await user.click(screen.getByRole("button", { name: "Send" }))
  const signal = vi.mocked(askArticleQuestion).mock.calls[0][3]
  await user.click(screen.getByRole("button", { name: "Stop" }))
  expect(signal?.aborted).toBe(true)
  expect(screen.getByRole("textbox", { name: "Your question" })).toHaveValue("Explain this")
  await user.click(screen.getByRole("button", { name: "Send" }))
  const second = vi.mocked(askArticleQuestion).mock.calls[1][3]
  rendered.rerender(view({ entryId: "entry-b" }))
  expect(second?.aborted).toBe(true)
  await act(async () => { resolve(answer) })
  expect(screen.queryByText("Codex", { exact: false })).not.toBeInTheDocument()
  expect(screen.getByRole("textbox", { name: "Your question" })).toHaveValue("")
})

it("uses the existing textarea and leaves IME composition and ordinary Enter untouched", async () => {
  const user = userEvent.setup()
  vi.mocked(askArticleQuestion).mockResolvedValue(answer)
  render(view())
  const input = screen.getByRole("textbox", { name: "Your question" })
  await user.type(input, "Explain")
  fireEvent.keyDown(input, { key: "Enter", ctrlKey: true, isComposing: true, keyCode: 229 })
  fireEvent.keyDown(input, { key: "Enter" })
  expect(askArticleQuestion).not.toHaveBeenCalled()
  fireEvent.keyDown(input, { key: "Enter", ctrlKey: true })
  await waitFor(() => expect(askArticleQuestion).toHaveBeenCalledOnce())
  const response = await screen.findByText("helps teams build tools.", { exact: false })
  expect(response.closest(".article-question-answer")).toHaveAttribute("data-animate", "false")
})

it("suggests a question without sending and enforces the input limit", async () => {
  const user = userEvent.setup()
  render(view())
  await user.click(screen.getByRole("button", { name: "Explain key concepts" }))
  const input = screen.getByRole("textbox", { name: "Your question" })
  expect(input).toHaveFocus()
  expect(askArticleQuestion).not.toHaveBeenCalled()
  fireEvent.change(input, { target: { value: "x".repeat(2001) } })
  expect(screen.getByRole("button", { name: "Send" })).toBeDisabled()
})

it.each(["failure", "stop"])("retains question A after %s while drafting B and retries A without losing B", async (outcome) => {
  const user = userEvent.setup()
  let reject!: (reason: Error) => void
  vi.mocked(askArticleQuestion).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail })).mockResolvedValue(answer)
  render(view())
  const input = screen.getByRole("textbox", { name: "Your question" })
  await user.type(input, "Question A")
  await user.click(screen.getByRole("button", { name: "Send" }))
  await user.type(input, "Question B")
  if (outcome === "failure") await act(async () => { reject(new Error("offline")) })
  else await user.click(screen.getByRole("button", { name: "Stop" }))
  expect(input).toHaveValue("Question B")
  expect(screen.getByText("Question A")).toBeInTheDocument()
  await user.click(screen.getByRole("button", { name: "Retry" }))
  await screen.findByText("helps teams build tools.", { exact: false })
  expect(vi.mocked(askArticleQuestion).mock.calls[1][2].question).toBe("Question A")
  expect(input).toHaveValue("Question B")
})
