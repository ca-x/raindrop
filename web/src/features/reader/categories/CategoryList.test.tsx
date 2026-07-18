import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { initialReaderState } from "../model/reducer"
import {
  categoryId,
  makeCategory,
  makeSubscription,
  subscriptionId,
} from "../model/testFixtures"
import { CategoryList } from "./CategoryList"

it("renders empty categories, categorized feeds, and Uncategorized in one TreeList", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onSelect = vi.fn()
  const emptyCategoryId = "00000000-0000-4000-8000-000000000502"
  render(
    <Providers>
      <CategoryList
        state={{
          ...initialReaderState,
          categoriesById: {
            [categoryId]: makeCategory(),
            [emptyCategoryId]: makeCategory({
              categoryId: emptyCategoryId,
              title: "Empty category",
              position: 2048,
            }),
          },
          categoryOrder: [categoryId, emptyCategoryId],
          subscriptionsById: {
            [subscriptionId]: makeSubscription({ categoryId, unreadCount: 7 }),
          },
          subscriptionOrder: [subscriptionId],
        }}
        onSelect={onSelect}
      />
    </Providers>,
  )

  expect(screen.getByText("Empty category")).toBeVisible()
  expect(screen.getByText("Uncategorized")).toBeVisible()
  expect(screen.getByText("Example Feed")).toBeVisible()

  await user.click(screen.getByRole("button", { name: "Technology" }))
  expect(onSelect).toHaveBeenCalledWith({ kind: "category", categoryId })
  await user.click(screen.getByRole("button", { name: "Example Feed" }))
  expect(onSelect).toHaveBeenCalledWith({ kind: "feed", feedId: makeSubscription().feedId })
})
