import { expect, it } from "vitest"

import { makeCategory, makeSubscription } from "../model/testFixtures"
import { groupSubscriptions } from "./groupSubscriptions"

it("preserves empty categories and derives stable aggregate unread counts", () => {
  const technology = makeCategory()
  const science = makeCategory({
    categoryId: "00000000-0000-4000-8000-000000000502",
    title: "Science",
    position: 2048,
  })
  const grouped = groupSubscriptions(
    [technology, science],
    [
      makeSubscription({ categoryId: technology.categoryId, unreadCount: 3 }),
      makeSubscription({
        subscriptionId: "00000000-0000-4000-8000-000000000202",
        categoryId: technology.categoryId,
        unreadCount: 4,
      }),
      makeSubscription({
        subscriptionId: "00000000-0000-4000-8000-000000000203",
        categoryId: null,
        unreadCount: 2,
      }),
      makeSubscription({
        subscriptionId: "00000000-0000-4000-8000-000000000204",
        categoryId: "00000000-0000-4000-8000-000000000599",
        unreadCount: 1,
      }),
    ],
  )

  expect(grouped.categorized.map((group) => group.category?.title)).toEqual([
    "Technology",
    "Science",
  ])
  expect(grouped.categorized[0]).toMatchObject({ unreadCount: 7 })
  expect(grouped.categorized[1]).toMatchObject({ subscriptions: [], unreadCount: 0 })
  expect(grouped.uncategorized.subscriptions).toHaveLength(2)
  expect(grouped.uncategorized.unreadCount).toBe(3)
})
