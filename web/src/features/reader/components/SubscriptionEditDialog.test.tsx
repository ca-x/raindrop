import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { categoryId, makeCategory, makeSubscription } from "../model/testFixtures"
import { SubscriptionEditDialog } from "./SubscriptionEditDialog"

it("updates the address and name, moves, opens, and deletes the selected feed", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const subscription = makeSubscription({
    categoryId,
    siteUrl: "https://publisher.example/",
  })
  const onUpdate = vi.fn(async () => true)
  const onDelete = vi.fn(async (_subscriptionId: string) => true)
  const onOpenChange = vi.fn()
  const onRequestMarkRead = vi.fn()

  render(
    <Providers>
      <SubscriptionEditDialog
        isOpen
        subscription={subscription}
        categories={[makeCategory()]}
        mutationError={null}
        linkOpenMode="NEW_TAB"
        onOpenChange={onOpenChange}
        onClearError={vi.fn()}
        onUpdate={onUpdate}
        onDelete={onDelete}
        onRequestMarkRead={onRequestMarkRead}
        isMarkingRead={false}
      />
    </Providers>,
  )

  const dialog = screen.getByRole("dialog", { name: "Edit current subscription" })
  const site = within(dialog).getByRole("link", { name: "https://publisher.example/" })
  expect(site).toHaveAttribute("href", "https://publisher.example/")
  expect(site).toHaveAttribute("target", "_blank")

  const feedUrl = within(dialog).getByRole("textbox", { name: /^Feed URL/ })
  expect(feedUrl).toHaveValue(subscription.feedUrl)
  await user.clear(feedUrl)
  await user.type(feedUrl, "https://publisher.example/new-feed.xml")

  const title = within(dialog).getByRole("textbox", { name: /^Custom name/ })
  await user.type(title, "Daily")
  await user.click(within(dialog).getByRole("button", { name: "Save feed" }))
  expect(onUpdate).toHaveBeenCalledWith(subscription.subscriptionId, {
    feedUrl: "https://publisher.example/new-feed.xml",
    titleOverride: "Daily",
  })
  expect(onOpenChange).toHaveBeenCalledWith(false)

  const category = within(dialog).getByRole("combobox", {
    name: /^Category for the current feed/,
  })
  await user.click(category)
  await user.keyboard("{Home}{Enter}")
  expect(onUpdate).toHaveBeenCalledWith(subscription.subscriptionId, {
    categoryId: null,
  })

  await user.click(within(dialog).getByRole("button", { name: "Mark current source read" }))
  expect(onRequestMarkRead).toHaveBeenCalledOnce()

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

it("keeps an invalid edited feed URL in place and explains the constraint", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onUpdate = vi.fn(async () => true)
  render(
    <Providers>
      <SubscriptionEditDialog
        isOpen
        subscription={makeSubscription()}
        categories={[]}
        mutationError={null}
        linkOpenMode="CURRENT_TAB"
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onUpdate={onUpdate}
        onDelete={vi.fn(async () => true)}
        onRequestMarkRead={vi.fn()}
        isMarkingRead={false}
      />
    </Providers>,
  )

  const feedUrl = screen.getByRole("textbox", { name: /^Feed URL/ })
  await user.clear(feedUrl)
  await user.type(feedUrl, "http://publisher.example/feed.xml")
  await user.click(screen.getByRole("button", { name: "Save feed" }))

  expect(feedUrl).toHaveValue("http://publisher.example/feed.xml")
  expect(screen.getByText("Enter an HTTPS feed URL.")).toBeVisible()
  expect(onUpdate).not.toHaveBeenCalled()
})

it("keeps the edited feed URL in place when saving fails", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onUpdate = vi.fn(async () => false)
  render(
    <Providers>
      <SubscriptionEditDialog
        isOpen
        subscription={makeSubscription()}
        categories={[]}
        mutationError={null}
        linkOpenMode="CURRENT_TAB"
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onUpdate={onUpdate}
        onDelete={vi.fn(async () => true)}
        onRequestMarkRead={vi.fn()}
        isMarkingRead={false}
      />
    </Providers>,
  )

  const feedUrl = screen.getByRole("textbox", { name: /^Feed URL/ })
  await user.clear(feedUrl)
  await user.type(feedUrl, "https://publisher.example/retry.xml")
  await user.click(screen.getByRole("button", { name: "Save feed" }))

  expect(onUpdate).toHaveBeenCalledOnce()
  expect(feedUrl).toHaveValue("https://publisher.example/retry.xml")
})

it("uses the current page for a feed site when that preference is selected", () => {
  activateLocale("en")
  render(
    <Providers>
      <SubscriptionEditDialog
        isOpen
        subscription={makeSubscription({ siteUrl: "https://publisher.example/" })}
        categories={[]}
        mutationError={null}
        linkOpenMode="CURRENT_TAB"
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onUpdate={vi.fn(async () => true)}
        onDelete={vi.fn(async () => true)}
        onRequestMarkRead={vi.fn()}
        isMarkingRead={false}
      />
    </Providers>,
  )

  expect(screen.getByRole("link", { name: "https://publisher.example/" })).not.toHaveAttribute(
    "target",
  )
})
