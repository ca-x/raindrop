import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { beforeEach, expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import TamboConversation from "./TamboConversation"

const sdk = vi.hoisted(() => ({
  provider: vi.fn(), cancelRun: vi.fn().mockResolvedValue(undefined), submit: vi.fn(),
  setValue: vi.fn(), streaming: false, error: false,
}))

vi.mock("@tambo-ai/react", () => ({
  TamboProvider: (props: { children: ReactNode }) => { sdk.provider(props); return props.children },
  useTambo: () => ({ messages: [], isStreaming: sdk.streaming, isWaiting: false, cancelRun: sdk.cancelRun }),
  useTamboThreadInput: () => ({ value: "Explain this article", setValue: sdk.setValue, submit: sdk.submit, isPending: false, isDisabled: false, isError: sdk.error }),
}))

beforeEach(() => {
  activateLocale("en")
  sdk.provider.mockClear()
  sdk.cancelRun.mockClear().mockResolvedValue(undefined)
  sdk.submit.mockReset().mockResolvedValue(undefined)
  sdk.streaming = false
  sdk.error = false
})

it("uses only the same-origin proxy and bounds article-only context; unmount cancels the run", () => {
  const article = { title: "t".repeat(700), text: "a".repeat(15000) }
  const view = render(<Providers><TamboConversation csrfToken="local-csrf" article={article} /></Providers>)
  const props = sdk.provider.mock.calls[0][0] as {
    apiKey: string; tamboUrl: string; userKey: string;
    contextHelpers: { article: () => { title: string; text: string; untrusted: boolean } }
  }
  expect(props.apiKey).toBe("local-csrf")
  expect(props.tamboUrl).toBe(`${window.location.origin}/api/v1/ai/tambo/api`)
  expect(props.userKey).toBe("authenticated-session")
  expect(props.contextHelpers.article()).toEqual({ title: "t".repeat(500), text: "a".repeat(12000), untrusted: true })
  expect(sdk.submit).not.toHaveBeenCalled()
  view.unmount()
  expect(sdk.cancelRun).toHaveBeenCalledOnce()
})

it("keeps send failures in the sidecar and allows stopping a streamed response", async () => {
  const user = userEvent.setup()
  sdk.submit.mockRejectedValue(new Error("offline"))
  const view = render(<Providers><TamboConversation csrfToken="csrf" article={{ title: "Title", text: "Text" }} /></Providers>)
  await user.click(screen.getByRole("button", { name: "Send" }))
  expect(await screen.findByText("Interactive assistant unavailable")).toBeVisible()
  sdk.streaming = true
  view.rerender(<Providers><TamboConversation csrfToken="csrf" article={{ title: "Title", text: "Text" }} /></Providers>)
  expect(screen.getByRole("textbox")).toBeDisabled()
  await user.click(screen.getByRole("button", { name: "Stop" }))
  expect(sdk.cancelRun).toHaveBeenCalledOnce()
})
