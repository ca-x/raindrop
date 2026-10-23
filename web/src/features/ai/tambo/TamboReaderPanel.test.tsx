import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { TamboReaderPanel } from "./TamboReaderPanel"

const loaded = vi.hoisted(() => vi.fn())
vi.mock("./TamboConversation", () => {
  loaded()
  return { default: () => <div>Conversation mounted</div> }
})

it("discloses the destination and defers the SDK until explicit consent", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onClose = vi.fn()
  render(<Providers><TamboReaderPanel csrfToken="csrf" origin="https://tambo.example"
    article={{ title: "Article", text: "Private article text" }} onClose={onClose} /></Providers>)
  expect(screen.getByText(/12,000 characters/)).toHaveTextContent("https://tambo.example")
  expect(screen.queryByText("Private article text")).not.toBeInTheDocument()
  expect(loaded).not.toHaveBeenCalled()
  expect(screen.getByRole("heading", { name: "Interactive reading" })).toHaveFocus()
  await user.click(screen.getByRole("button", { name: "Start conversation" }))
  expect(await screen.findByText("Conversation mounted")).toBeVisible()
  expect(loaded).toHaveBeenCalledOnce()
  await user.keyboard("{Escape}")
  expect(onClose).toHaveBeenCalledOnce()
})
