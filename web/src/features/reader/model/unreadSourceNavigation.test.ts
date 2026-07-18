import { expect, it } from "vitest"

import { initialReaderState } from "./reducer"
import { makeCategory, makeSubscription } from "./testFixtures"
import { adjacentUnreadSource } from "./unreadSourceNavigation"
import type { ReaderState } from "./types"

const categoryA = "00000000-0000-4000-8000-000000000501"
const categoryB = "00000000-0000-4000-8000-000000000502"
const feedA = "00000000-0000-4000-8000-000000000101"
const feedB = "00000000-0000-4000-8000-000000000102"
const feedC = "00000000-0000-4000-8000-000000000103"

it("walks unread Feeds in rendered category then uncategorized order", () => {
  const state = navigationState()
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "smart", state: "ALL" } },
      1,
    ),
  ).toEqual({ kind: "feed", feedId: feedA })
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "feed", feedId: feedA } },
      1,
    ),
  ).toEqual({ kind: "feed", feedId: feedC })
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "feed", feedId: feedC } },
      -1,
    ),
  ).toEqual({ kind: "feed", feedId: feedA })
})

it("starts inside a selected Category and falls back to Unread without wrapping", () => {
  const state = navigationState()
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "category", categoryId: categoryB } },
      -1,
    ),
  ).toEqual({ kind: "feed", feedId: feedA })
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "category", categoryId: categoryB } },
      1,
    ),
  ).toEqual({ kind: "feed", feedId: feedC })
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "feed", feedId: feedC } },
      1,
    ),
  ).toEqual({ kind: "smart", state: "UNREAD" })
  expect(
    adjacentUnreadSource(
      { ...state, selectedSource: { kind: "smart", state: "UNREAD" } },
      -1,
    ),
  ).toEqual({ kind: "feed", feedId: feedC })
})

it("returns no action when Unread is selected and no Feed has unread entries", () => {
  const state = navigationState()
  const subscriptionsById = Object.fromEntries(
    Object.entries(state.subscriptionsById).map(([id, subscription]) => [
      id,
      { ...subscription, unreadCount: 0 },
    ]),
  )
  expect(
    adjacentUnreadSource(
      {
        ...state,
        subscriptionsById,
        selectedSource: { kind: "smart", state: "UNREAD" },
      },
      1,
    ),
  ).toBeNull()
})

function navigationState(): ReaderState {
  const subscriptionA = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000201",
    feedId: feedA,
    categoryId: categoryA,
    unreadCount: 2,
  })
  const subscriptionB = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000202",
    feedId: feedB,
    categoryId: categoryB,
    unreadCount: 0,
  })
  const subscriptionC = makeSubscription({
    subscriptionId: "00000000-0000-4000-8000-000000000203",
    feedId: feedC,
    categoryId: null,
    unreadCount: 4,
  })
  return {
    ...structuredClone(initialReaderState),
    categoriesById: {
      [categoryA]: makeCategory({ categoryId: categoryA, position: 1024 }),
      [categoryB]: makeCategory({ categoryId: categoryB, position: 2048 }),
    },
    categoryOrder: [categoryA, categoryB],
    subscriptionsById: {
      [subscriptionA.subscriptionId]: subscriptionA,
      [subscriptionB.subscriptionId]: subscriptionB,
      [subscriptionC.subscriptionId]: subscriptionC,
    },
    subscriptionOrder: [
      subscriptionC.subscriptionId,
      subscriptionB.subscriptionId,
      subscriptionA.subscriptionId,
    ],
  }
}
