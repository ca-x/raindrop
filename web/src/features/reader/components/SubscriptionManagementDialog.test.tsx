import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { makeSubscription } from "../model/testFixtures"
import { SubscriptionManagementDialog } from "./SubscriptionManagementDialog"

it("keeps the new-feed draft across tabs and deletes the created feed before returning to its URL", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const created = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000711",
    feedUrl: "https://publisher.example/feed.xml",
    title: "Publisher",
  })
  const onDelete = vi.fn(async (_subscriptionId: string) => true)

  function Harness() {
    const [subscriptions, setSubscriptions] = useState<ReturnType<typeof makeSubscription>[]>([])
    return (
      <Providers>
        <SubscriptionManagementDialog
          isOpen
          subscriptions={subscriptions}
          categories={[]}
          mutationError={null}
          csrfToken="csrf-memory"
          onOpenChange={vi.fn()}
          onClearError={vi.fn()}
          onAdd={async () => {
            setSubscriptions([created])
            return { created: true, subscription: created }
          }}
          onUpdate={vi.fn(async () => true)}
          onDelete={async (subscriptionId) => {
            const deleted = await onDelete(subscriptionId)
            if (deleted) setSubscriptions([])
            return deleted
          }}
          onCreateCategory={vi.fn(async () => true)}
          onUpdateCategory={vi.fn(async () => true)}
          onDeleteCategory={vi.fn(async () => true)}
          onSubscriptionsChanged={vi.fn()}
        />
      </Providers>
    )
  }

  render(<Harness />)
  const dialog = screen.getByRole("dialog", { name: "Manage subscriptions" })
  await user.type(
    within(dialog).getByRole("textbox", { name: /^Feed URL/ }),
    created.feedUrl,
  )
  await user.click(within(dialog).getByRole("button", { name: "Continue" }))
  await user.type(
    within(dialog).getByRole("textbox", { name: "Custom name" }),
    "Daily",
  )

  await user.click(within(dialog).getByRole("button", { name: "Add category" }))
  await user.click(within(dialog).getByRole("button", { name: "Subscriptions" }))
  expect(within(dialog).getByRole("textbox", { name: "Custom name" })).toHaveValue(
    "Daily",
  )

  await user.click(within(dialog).getByRole("button", { name: "Back to URL" }))
  expect(onDelete).toHaveBeenCalledWith(created.subscriptionId)
  expect(within(dialog).getByRole("textbox", { name: /^Feed URL/ })).toBeEnabled()
})

it("never deletes an existing subscription when returning to its URL", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const existing = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000712",
    feedUrl: "https://publisher.example/feed.xml",
    title: "Publisher",
  })
  const onDelete = vi.fn(async (_subscriptionId: string) => true)

  render(
    <Providers>
      <SubscriptionManagementDialog
        isOpen
        subscriptions={[existing]}
        categories={[]}
        mutationError={null}
        csrfToken="csrf-memory"
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onAdd={vi.fn(async () => ({ created: false, subscription: existing }))}
        onUpdate={vi.fn(async () => true)}
        onDelete={onDelete}
        onCreateCategory={vi.fn(async () => true)}
        onUpdateCategory={vi.fn(async () => true)}
        onDeleteCategory={vi.fn(async () => true)}
        onSubscriptionsChanged={vi.fn()}
      />
    </Providers>,
  )

  const dialog = screen.getByRole("dialog", { name: "Manage subscriptions" })
  await user.type(
    within(dialog).getByRole("textbox", { name: /^Feed URL/ }),
    existing.feedUrl,
  )
  await user.click(within(dialog).getByRole("button", { name: "Continue" }))
  expect(within(dialog).getByText(/already subscribed/u)).toBeVisible()

  await user.click(within(dialog).getByRole("button", { name: "Back to URL" }))
  expect(onDelete).not.toHaveBeenCalled()
  expect(within(dialog).getByRole("textbox", { name: /^Feed URL/ })).toBeEnabled()
})
