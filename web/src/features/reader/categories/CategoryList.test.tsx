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
        density="balanced"
      />
    </Providers>,
  )

  expect(screen.getByText("Empty category")).toBeVisible()
  expect(screen.getByText("Uncategorized")).toBeVisible()
  expect(screen.getByText("Example Feed")).toBeVisible()
  const favicon = document.querySelector<HTMLImageElement>(".reader-source-favicon")!
  expect(favicon).toHaveAttribute(
    "src",
    `/reader-assets/subscriptions/${subscriptionId}/favicon`,
  )
  expect(favicon).toHaveAttribute("loading", "lazy")
  expect(favicon).toHaveAttribute("referrerpolicy", "no-referrer")
  expect(screen.getByRole("tree").parentElement).toHaveAttribute(
    "data-density",
    "balanced",
  )

  await user.click(screen.getByRole("button", { name: "Technology" }))
  expect(onSelect).toHaveBeenCalledWith({ kind: "category", categoryId })
  await user.click(screen.getByRole("button", { name: "Example Feed" }))
  expect(onSelect).toHaveBeenCalledWith({ kind: "feed", feedId: makeSubscription().feedId })
})

it.each([
  ["QUEUED", "Queued for refresh"],
  ["RUNNING", "Refreshing"],
] as const)("announces %s refresh activity distinctly", (pendingState, label) => {
  activateLocale("en")
  render(
    <Providers>
      <CategoryList
        state={{
          ...initialReaderState,
          subscriptionsById: {
            [subscriptionId]: makeSubscription({
              refresh: {
                operationId: "00000000-0000-4000-8000-000000000401",
                state: "PENDING",
                pendingState,
                newCount: 0,
                updatedCount: 0,
                droppedCount: 0,
                entryIssues: [],
                generation: null,
                errorCode: null,
                retryAt: null,
                lastSuccessAt: null,
                queuedAt: "2026-07-18T02:00:00.000000Z",
                startedAt:
                  pendingState === "RUNNING" ? "2026-07-18T02:00:01.000000Z" : null,
                completedAt: null,
              },
            }),
          },
          subscriptionOrder: [subscriptionId],
        }}
        onSelect={vi.fn()}
        density="balanced"
      />
    </Providers>,
  )

  expect(screen.getByLabelText(label)).toBeVisible()
})

it("filters subscriptions without hiding the smart views", () => {
  activateLocale("en")
  const secondSubscriptionId = "00000000-0000-4000-8000-000000000202"
  render(
    <Providers>
      <CategoryList
        state={{
          ...initialReaderState,
          categoriesById: { [categoryId]: makeCategory() },
          categoryOrder: [categoryId],
          subscriptionsById: {
            [subscriptionId]: makeSubscription({ categoryId, title: "Rust Blog" }),
            [secondSubscriptionId]: makeSubscription({
              subscriptionId: secondSubscriptionId,
              feedId: "00000000-0000-4000-8000-000000000102",
              categoryId,
              title: "Design Notes",
            }),
          },
          subscriptionOrder: [subscriptionId, secondSubscriptionId],
        }}
        onSelect={vi.fn()}
        density="balanced"
        query="rust"
      />
    </Providers>,
  )

  expect(screen.getByText("Rust Blog")).toBeVisible()
  expect(screen.queryByText("Design Notes")).not.toBeInTheDocument()
  expect(screen.getByRole("button", { name: "Unread" })).toBeVisible()
})
