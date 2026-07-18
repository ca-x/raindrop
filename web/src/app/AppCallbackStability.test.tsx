import { act, render, screen, waitFor } from "@testing-library/react"
import { beforeEach, expect, it, vi } from "vitest"

import { activateLocale } from "../shared/i18n/i18n"
import { App } from "./App"
import { Providers } from "./Providers"

const readyPageCallbacks = vi.hoisted(() => [] as Array<() => void>)

vi.mock("./useInitialAppState", () => ({
  useInitialAppState: () => ({
    status: "ready",
    value: {
      phase: "ready",
      session: {
        csrfToken: "csrf-memory",
        user: { userId: "user-memory", username: "Reader" },
      },
    },
  }),
}))

vi.mock("../features/reader/ReadyPage", () => ({
  ReadyPage: ({ onLoggedOut }: { onLoggedOut: () => void }) => {
    readyPageCallbacks.push(onLoggedOut)
    return <div>Ready page stub</div>
  },
}))

beforeEach(() => {
  readyPageCallbacks.length = 0
  activateLocale("en")
})

it("keeps the logout transition stable across locale-driven app renders", async () => {
  render(
    <Providers>
      <App />
    </Providers>,
  )
  expect(await screen.findByText("Ready page stub")).toBeVisible()
  const initialCallback = readyPageCallbacks.at(-1)

  act(() => activateLocale("zh-CN"))
  await waitFor(() => expect(readyPageCallbacks.length).toBeGreaterThan(1))

  expect(readyPageCallbacks.at(-1)).toBe(initialCallback)
})
