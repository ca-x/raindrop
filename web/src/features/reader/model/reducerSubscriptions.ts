import type { Category } from "../api/organization.generated"
import type { Refresh, Subscription } from "../api/subscription.generated"
import { reconcileSubscription } from "./reducerOptimistic"
import { sortedCategoryIds } from "./reducerCategories"
import { sourceKey, type ReaderState } from "./types"

export function requestSubscriptions(
  state: ReaderState,
  generation: number,
): ReaderState {
  return {
    ...state,
    requestGenerationByPane: {
      ...state.requestGenerationByPane,
      subscriptions: generation,
    },
    paneStatus: { ...state.paneStatus, subscriptions: "loading" },
    errors: { ...state.errors, subscriptions: null },
  }
}

export function receiveSubscriptions(
  state: ReaderState,
  generation: number,
  subscriptions: Subscription[],
  categories: Category[],
): ReaderState {
  if (generation !== state.requestGenerationByPane.subscriptions) return state
  const reconciled = subscriptions.map((subscription) =>
    reconcileSubscription(state, subscription),
  )
  return {
    ...state,
    categoriesById: Object.fromEntries(
      categories.map((category) => [category.categoryId, category]),
    ),
    categoryOrder: sortedCategoryIds(
      Object.fromEntries(categories.map((category) => [category.categoryId, category])),
    ),
    subscriptionsById: Object.fromEntries(
      reconciled.map((subscription) => [subscription.subscriptionId, subscription]),
    ),
    subscriptionOrder: reconciled.map((subscription) => subscription.subscriptionId),
    paneStatus: { ...state.paneStatus, subscriptions: "ready" },
    errors: { ...state.errors, subscriptions: null },
  }
}

export function failSubscriptions(
  state: ReaderState,
  generation: number,
  error: string,
): ReaderState {
  if (generation !== state.requestGenerationByPane.subscriptions) return state
  return {
    ...state,
    paneStatus: { ...state.paneStatus, subscriptions: "error" },
    errors: { ...state.errors, subscriptions: error },
  }
}

export function upsertSubscription(
  state: ReaderState,
  subscription: Subscription,
): ReaderState {
  const reconciled = reconcileSubscription(state, subscription)
  const exists = subscription.subscriptionId in state.subscriptionsById
  return {
    ...state,
    subscriptionsById: {
      ...state.subscriptionsById,
      [subscription.subscriptionId]: reconciled,
    },
    subscriptionOrder: exists
      ? state.subscriptionOrder
      : [...state.subscriptionOrder, subscription.subscriptionId],
    errors: { ...state.errors, mutation: null },
  }
}

export function deleteSubscriptionState(
  state: ReaderState,
  subscriptionId: string,
): ReaderState {
  const subscription = state.subscriptionsById[subscriptionId]
  if (!subscription) return state
  const subscriptionsById = { ...state.subscriptionsById }
  delete subscriptionsById[subscriptionId]
  const feedKey = sourceKey({ kind: "feed", feedId: subscription.feedId })
  const queueBySourceKey = { ...state.queueBySourceKey }
  const pendingNewEntriesBySource = { ...state.pendingNewEntriesBySource }
  const pendingNewEntryCountBySource = { ...state.pendingNewEntryCountBySource }
  delete queueBySourceKey[feedKey]
  delete pendingNewEntriesBySource[feedKey]
  delete pendingNewEntryCountBySource[feedKey]
  const selectedDeletedFeed =
    state.selectedSource.kind === "feed" &&
    state.selectedSource.feedId === subscription.feedId
  return {
    ...state,
    subscriptionsById,
    subscriptionOrder: state.subscriptionOrder.filter((id) => id !== subscriptionId),
    queueBySourceKey,
    pendingNewEntriesBySource,
    pendingNewEntryCountBySource,
    selectedSource: selectedDeletedFeed
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedDeletedFeed ? null : state.selectedEntryId,
    errors: { ...state.errors, mutation: null },
  }
}

export function updateSubscriptionRefresh(
  state: ReaderState,
  subscriptionId: string,
  refresh: Refresh,
): ReaderState {
  const subscription = state.subscriptionsById[subscriptionId]
  if (!subscription) return state
  return {
    ...state,
    subscriptionsById: {
      ...state.subscriptionsById,
      [subscriptionId]: { ...subscription, refresh },
    },
    errors: { ...state.errors, mutation: null },
  }
}
