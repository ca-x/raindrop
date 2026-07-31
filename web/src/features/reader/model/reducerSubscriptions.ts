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
  const retiredFeedIds = { ...state.retiredFeedIds }
  for (const subscription of reconciled) delete retiredFeedIds[subscription.feedId]
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
    retiredFeedIds,
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
  const previous = state.subscriptionsById[subscription.subscriptionId]
  const exists = previous !== undefined
  const feedChanged = previous !== undefined && previous.feedId !== subscription.feedId
  const previousFeedKey = previous
    ? sourceKey({ kind: "feed", feedId: previous.feedId })
    : null
  const selectedPreviousFeed = Boolean(
    feedChanged &&
    previous &&
    state.selectedSource.kind === "feed" &&
    state.selectedSource.feedId === previous.feedId,
  )
  const retiredFeedIds = { ...state.retiredFeedIds }
  delete retiredFeedIds[subscription.feedId]
  if (feedChanged && previous) retiredFeedIds[previous.feedId] = true
  return {
    ...state,
    subscriptionsById: {
      ...state.subscriptionsById,
      [subscription.subscriptionId]: reconciled,
    },
    subscriptionOrder: exists
      ? state.subscriptionOrder
      : [...state.subscriptionOrder, subscription.subscriptionId],
    retiredFeedIds,
    queueBySourceKey: withoutSourceKey(state.queueBySourceKey, feedChanged ? previousFeedKey : null),
    pendingNewEntriesBySource: withoutSourceKey(
      state.pendingNewEntriesBySource,
      feedChanged ? previousFeedKey : null,
    ),
    pendingNewEntryCountBySource: withoutSourceKey(
      state.pendingNewEntryCountBySource,
      feedChanged ? previousFeedKey : null,
    ),
    snapshotGenerationBySource: withoutSourceKey(
      state.snapshotGenerationBySource,
      feedChanged ? previousFeedKey : null,
    ),
    pendingSnapshotGenerationBySource: withoutSourceKey(
      state.pendingSnapshotGenerationBySource,
      feedChanged ? previousFeedKey : null,
    ),
    selectedSource: selectedPreviousFeed
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedPreviousFeed ? null : state.selectedEntryId,
    feedSearchQuery: selectedPreviousFeed ? "" : state.feedSearchQuery,
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
  const snapshotGenerationBySource = { ...state.snapshotGenerationBySource }
  const pendingSnapshotGenerationBySource = {
    ...state.pendingSnapshotGenerationBySource,
  }
  delete queueBySourceKey[feedKey]
  delete pendingNewEntriesBySource[feedKey]
  delete pendingNewEntryCountBySource[feedKey]
  delete snapshotGenerationBySource[feedKey]
  delete pendingSnapshotGenerationBySource[feedKey]
  const selectedDeletedFeed =
    state.selectedSource.kind === "feed" &&
    state.selectedSource.feedId === subscription.feedId
  return {
    ...state,
    subscriptionsById,
    subscriptionOrder: state.subscriptionOrder.filter((id) => id !== subscriptionId),
    retiredFeedIds: { ...state.retiredFeedIds, [subscription.feedId]: true },
    queueBySourceKey,
    pendingNewEntriesBySource,
    pendingNewEntryCountBySource,
    snapshotGenerationBySource,
    pendingSnapshotGenerationBySource,
    selectedSource: selectedDeletedFeed
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedDeletedFeed ? null : state.selectedEntryId,
    feedSearchQuery: selectedDeletedFeed ? "" : state.feedSearchQuery,
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

function withoutSourceKey<T>(
  values: Partial<Record<ReturnType<typeof sourceKey>, T>>,
  key: ReturnType<typeof sourceKey> | null,
): Partial<Record<ReturnType<typeof sourceKey>, T>> {
  if (!key || !(key in values)) return values
  const next = { ...values }
  delete next[key]
  return next
}
