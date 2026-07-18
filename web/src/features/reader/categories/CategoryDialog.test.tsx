import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { categoryId, makeCategory, makeSubscription } from "../model/testFixtures"
import { CategoryDialog } from "./CategoryDialog"

it("creates, renames, assigns, and clears categories with ASTRYX form controls", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onCreate = vi.fn(async () => true)
  const onUpdate = vi.fn(async () => true)
  const onAssign = vi.fn(async () => true)
  const subscription = makeSubscription({ categoryId })
  render(
    <Providers>
      <CategoryDialog
        isOpen
        categories={[makeCategory()]}
        subscriptions={[subscription]}
        selectedSubscription={subscription}
        mutationError={null}
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onCreate={onCreate}
        onUpdate={onUpdate}
        onDelete={vi.fn(async () => true)}
        onAssign={onAssign}
      />
    </Providers>,
  )

  await user.click(screen.getByRole("button", { name: "Create category" }))
  expect(screen.getByText("Use a category name of 1 to 80 characters.")).toBeVisible()
  expect(onCreate).not.toHaveBeenCalled()

  const newCategory = screen.getByRole("textbox", { name: /^New category/ })
  await user.type(newCategory, "Science")
  await user.click(screen.getByRole("button", { name: "Create category" }))
  expect(onCreate).toHaveBeenCalledWith("Science")

  const categoryName = screen.getByRole("textbox", { name: /^Category name/ })
  await user.clear(categoryName)
  await user.type(categoryName, "Engineering")
  await user.click(screen.getByRole("button", { name: "Save changes" }))
  expect(onUpdate).toHaveBeenCalledWith(categoryId, "Engineering")

  const selector = screen.getByRole("combobox", {
    name: /^Category for the current feed/,
  })
  await user.click(selector)
  await user.keyboard("{Home}{Enter}")
  expect(onAssign).toHaveBeenCalledWith(subscription.subscriptionId, null)
})

it("uses a replacement AlertDialog for deletion and keeps mutation errors in the form", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onDelete = vi.fn(async () => true)
  render(
    <Providers>
      <CategoryDialog
        isOpen
        categories={[makeCategory()]}
        subscriptions={[makeSubscription({ categoryId })]}
        mutationError="Category name already exists."
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onCreate={vi.fn(async () => true)}
        onUpdate={vi.fn(async () => true)}
        onDelete={onDelete}
        onAssign={vi.fn(async () => true)}
      />
    </Providers>,
  )

  expect(screen.getByText("Category name already exists.")).toBeVisible()
  await user.click(screen.getByRole("button", { name: "Delete category" }))
  const alert = await screen.findByRole("alertdialog", {
    name: "Delete this category?",
  })
  expect(within(alert).getByText(/1 subscriptions will move to Uncategorized/)).toBeVisible()
  await user.click(within(alert).getByRole("button", { name: "Delete category" }))
  expect(onDelete).toHaveBeenCalledWith(categoryId)
})

it("restores the server assignment when moving a feed fails", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const subscription = makeSubscription({ categoryId })
  const onAssign = vi.fn(async () => false)
  render(
    <Providers>
      <CategoryDialog
        isOpen
        categories={[makeCategory()]}
        subscriptions={[subscription]}
        selectedSubscription={subscription}
        mutationError={null}
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onCreate={vi.fn(async () => true)}
        onUpdate={vi.fn(async () => true)}
        onDelete={vi.fn(async () => true)}
        onAssign={onAssign}
      />
    </Providers>,
  )

  const selector = screen.getByRole("combobox", {
    name: /^Category for the current feed/,
  })
  await user.click(selector)
  await user.keyboard("{Home}{Enter}")

  expect(onAssign).toHaveBeenCalledWith(subscription.subscriptionId, null)
  await waitFor(() => expect(selector).toHaveTextContent("Technology"))
})

it("returns to the form so a deletion error remains visible", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onDelete = vi.fn(async () => false)
  const { rerender } = render(
    <Providers>
      <CategoryDialog
        isOpen
        categories={[makeCategory()]}
        subscriptions={[]}
        mutationError={null}
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onCreate={vi.fn(async () => true)}
        onUpdate={vi.fn(async () => true)}
        onDelete={onDelete}
        onAssign={vi.fn(async () => true)}
      />
    </Providers>,
  )

  await user.click(screen.getByRole("button", { name: "Delete category" }))
  const alert = await screen.findByRole("alertdialog", {
    name: "Delete this category?",
  })
  await user.click(within(alert).getByRole("button", { name: "Delete category" }))

  rerender(
    <Providers>
      <CategoryDialog
        isOpen
        categories={[makeCategory()]}
        subscriptions={[]}
        mutationError="Category could not be deleted."
        onOpenChange={vi.fn()}
        onClearError={vi.fn()}
        onCreate={vi.fn(async () => true)}
        onUpdate={vi.fn(async () => true)}
        onDelete={onDelete}
        onAssign={vi.fn(async () => true)}
      />
    </Providers>,
  )

  const dialog = await screen.findByRole("dialog", { name: "Manage categories" })
  expect(within(dialog).getByText("Category could not be deleted.")).toBeVisible()
})
