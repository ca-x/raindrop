import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { categoryId, makeCategory, makeSubscription } from "../model/testFixtures"
import { SubscriptionEditDialog } from "./SubscriptionEditDialog"

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
      />
    </Providers>,
  )

  const dialog = screen.getByRole("dialog", { name: "Edit current subscription" })
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
      />
    </Providers>,
  )

  expect(screen.getByRole("link", { name: "https://publisher.example/" })).not.toHaveAttribute(
    "target",
  )
})
