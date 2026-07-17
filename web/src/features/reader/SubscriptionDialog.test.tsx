import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { describe, expect, it, vi } from "vitest"

import { Providers } from "../../app/Providers"
import { activateLocale } from "../../shared/i18n/i18n"
import { SubscriptionDialog } from "./components/SubscriptionDialog"

describe("SubscriptionDialog", () => {
  it("validates HTTPS feed URLs and retains input after a failed add", async () => {
    activateLocale("en")
    const user = userEvent.setup()
    const addSubscription = vi.fn().mockResolvedValue(undefined)

    render(
      <Providers>
        <DialogHarness addSubscription={addSubscription} />
      </Providers>,
    )

    const input = screen.getByRole("textbox", { name: /^Feed URL/ })
    await user.type(input, "http://feeds.example/rss")
    await user.click(screen.getByRole("button", { name: "Add subscription" }))
    expect(screen.getByText("Enter an HTTPS feed URL.")).toBeVisible()
    expect(addSubscription).not.toHaveBeenCalled()

    await user.clear(input)
    await user.type(input, "https://feeds.example/rss")
    await user.click(screen.getByRole("button", { name: "Add subscription" }))

    expect(addSubscription).toHaveBeenCalledWith("https://feeds.example/rss")
    expect(await screen.findByText("You cannot add this feed.")).toBeVisible()
    expect(input).toHaveValue("https://feeds.example/rss")
  })
})

function DialogHarness({ addSubscription }: { addSubscription: (url: string) => Promise<void> }) {
  const [error, setError] = useState<string | null>(null)
  return (
    <SubscriptionDialog
      isOpen
      mutationError={error}
      onOpenChange={vi.fn()}
      onClearError={() => setError(null)}
      onAdd={async (url) => {
        await addSubscription(url)
        setError("You cannot add this feed.")
      }}
    />
  )
}
