import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { categoryId, makeCategory, makeSubscription } from "../model/testFixtures"
import { SubscriptionManagementDialog } from "./SubscriptionManagementDialog"

it("renames, moves, opens, and deletes the selected feed", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const subscription = makeSubscription({
    categoryId,
    siteUrl: "https://publisher.example/",
  })
  const onUpdate = vi.fn(async () => true)
  const onDelete = vi.fn(async (_subscriptionId: string) => true)
  const onOpenChange = vi.fn()

  render(
    <Providers>
      <SubscriptionManagementDialog
        isOpen
        selectedSubscription={subscription}
        subscriptions={[subscription]}
        categories={[makeCategory()]}
        mutationError={null}
        linkOpenMode="NEW_TAB"
        csrfToken="csrf-memory"
        onOpenChange={onOpenChange}
        onClearError={vi.fn()}
        onAdd={vi.fn(async () => ({ created: true, subscription }))}
        onUpdate={onUpdate}
        onDelete={onDelete}
        onCreateCategory={vi.fn(async () => true)}
        onUpdateCategory={vi.fn(async () => true)}
        onDeleteCategory={vi.fn(async () => true)}
        onSubscriptionsChanged={vi.fn()}
      />
    </Providers>,
  )

  const dialog = screen.getByRole("dialog", { name: "Manage subscriptions" })
  const site = within(dialog).getByRole("link", { name: "https://publisher.example/" })
  expect(site).toHaveAttribute("href", "https://publisher.example/")
  expect(site).toHaveAttribute("target", "_blank")

  const title = within(dialog).getByRole("textbox", { name: /^Custom name/ })
  await user.type(title, "Daily")
  await user.click(within(dialog).getByRole("button", { name: "Save feed" }))
  expect(onUpdate).toHaveBeenCalledWith(subscription.subscriptionId, {
    titleOverride: "Daily",
  })

  const category = within(dialog).getByRole("combobox", {
    name: /^Category for the current feed/,
  })
  await user.click(category)
  await user.keyboard("{Home}{Enter}")
  expect(onUpdate).toHaveBeenCalledWith(subscription.subscriptionId, {
    categoryId: null,
  })

  await user.click(within(dialog).getByRole("button", { name: "Delete subscription" }))
  const alert = await screen.findByRole("alertdialog", {
    name: "Delete this subscription?",
  })
  await user.click(
    within(alert).getByRole("button", { name: "Delete subscription" }),
  )
  expect(onDelete).toHaveBeenCalledWith(subscription.subscriptionId)
  expect(onOpenChange).toHaveBeenCalledWith(false)
})

it("uses the current page for a feed site when that preference is selected", () => {
  activateLocale("en")
  render(
    <Providers>
      <SubscriptionManagementDialog
        isOpen
        selectedSubscription={makeSubscription({ siteUrl: "https://publisher.example/" })}
        subscriptions={[]}
        categories={[]}
        mutationError={null}
        linkOpenMode="CURRENT_TAB"
        csrfToken="csrf-memory"
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onAdd={vi.fn(async () => null)}
        onUpdate={vi.fn(async () => true)}
        onDelete={vi.fn(async () => true)}
        onCreateCategory={vi.fn(async () => true)}
        onUpdateCategory={vi.fn(async () => true)}
        onDeleteCategory={vi.fn(async () => true)}
        onSubscriptionsChanged={vi.fn()}
      />
    </Providers>,
  )

  expect(screen.getByRole("link", { name: "https://publisher.example/" })).not.toHaveAttribute(
    "target",
  )
})

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
          linkOpenMode="NEW_TAB"
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
        linkOpenMode="NEW_TAB"
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
