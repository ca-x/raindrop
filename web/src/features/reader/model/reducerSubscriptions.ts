import type { Category } from "../api/organization.generated"
import type { Refresh, Subscription } from "../api/subscription.generated"
import { reconcileSubscription } from "./reducerOptimistic"
import { sortedCategoryIds } from "./reducerCategories"
import { sourceKey, type ReaderState, type SourceKey } from "./types"

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
    paneStatus: {
      ...state.paneStatus,
      subscriptions:
        state.paneStatus.subscriptions === "ready" ? "ready" : "loading",
    },
    requestActivity: { ...state.requestActivity, subscriptions: true },
    errors: { ...state.errors, subscriptions: null },
  }
}

export function receiveSubscriptions(
  state: ReaderState,
  generation: number,
  subscriptions: Subscription[],
  categories: Category[],
  isFinal = true,
): ReaderState {
  if (generation !== state.requestGenerationByPane.subscriptions) return state
  const reconciled = subscriptions.map((subscription) =>
    reconcileSubscription(state, subscription),
  )
  const categoriesById = Object.fromEntries(
    categories.map((category) => [category.categoryId, category]),
  )
  const subscriptionsById = Object.fromEntries(
    reconciled.map((subscription) => [subscription.subscriptionId, subscription]),
  )
  if (!isFinal) {
    const progressiveCategoriesById = {
      ...state.categoriesById,
      ...categoriesById,
    }
    const progressiveSubscriptionsById = {
      ...state.subscriptionsById,
      ...subscriptionsById,
    }
    const receivedIds = reconciled.map((subscription) => subscription.subscriptionId)
    const receivedIdSet = new Set(receivedIds)
    return {
      ...state,
      categoriesById: progressiveCategoriesById,
      categoryOrder: sortedCategoryIds(progressiveCategoriesById),
      subscriptionsById: progressiveSubscriptionsById,
      subscriptionOrder: [
        ...receivedIds,
        ...state.subscriptionOrder.filter((id) => !receivedIdSet.has(id)),
      ],
      subscriptionsAuthoritative: false,
      paneStatus: { ...state.paneStatus, subscriptions: "ready" },
      requestActivity: { ...state.requestActivity, subscriptions: true },
      errors: { ...state.errors, subscriptions: null },
    }
  }
  const validFeedIds = new Set(reconciled.map((subscription) => subscription.feedId))
  const validCategoryIds = new Set(categories.map((category) => category.categoryId))
  const feedIdsByCategory = new Map<string, Set<string>>()
  for (const subscription of reconciled) {
    if (subscription.categoryId === null) continue
    const feedIds = feedIdsByCategory.get(subscription.categoryId) ?? new Set<string>()
    feedIds.add(subscription.feedId)
    feedIdsByCategory.set(subscription.categoryId, feedIds)
  }
  const changedMembershipKeys = state.subscriptionOrder.length === 0
    ? new Set<SourceKey>()
    : membershipChangedSourceKeys(
        Object.values(state.subscriptionsById),
        reconciled,
      )
  const selectedMembershipChanged = changedMembershipKeys.has(
    sourceKey(state.selectedSource),
  )
  const selectedSourceMissing =
    (state.selectedSource.kind === "feed" &&
      !validFeedIds.has(state.selectedSource.feedId)) ||
    (state.selectedSource.kind === "category" &&
      !validCategoryIds.has(state.selectedSource.categoryId))
  const retiredFeedIds = { ...state.retiredFeedIds }
  for (const previous of Object.values(state.subscriptionsById)) {
    if (!validFeedIds.has(previous.feedId)) retiredFeedIds[previous.feedId] = true
  }
  for (const subscription of reconciled) delete retiredFeedIds[subscription.feedId]
  const queueBySourceKey = withoutSourceKeys(
    pruneEntryQueues(
      state.queueBySourceKey,
      state,
      validFeedIds,
      validCategoryIds,
      feedIdsByCategory,
    ),
    changedMembershipKeys,
  )
  const pendingNewEntriesBySource = withoutSourceKeys(
    pruneEntryQueues(
      state.pendingNewEntriesBySource,
      state,
      validFeedIds,
      validCategoryIds,
      feedIdsByCategory,
    ),
    changedMembershipKeys,
  )
  return {
    ...state,
    categoriesById,
    categoryOrder: sortedCategoryIds(categoriesById),
    subscriptionsById,
    subscriptionOrder: reconciled.map((subscription) => subscription.subscriptionId),
    subscriptionsAuthoritative: true,
    retiredFeedIds,
    queueBySourceKey,
    nextCursorBySourceKey: withoutSourceKeys(
      pruneSourceRecord(
        state.nextCursorBySourceKey,
        validFeedIds,
        validCategoryIds,
      ),
      changedMembershipKeys,
    ),
    pendingNewEntriesBySource,
    pendingNewEntryCountBySource: Object.fromEntries(
      Object.entries(pendingNewEntriesBySource).map(([key, entryIds]) => [
        key,
        entryIds?.length ?? 0,
      ]),
    ),
    snapshotGenerationBySource: withoutSourceKeys(
      pruneSourceRecord(
        state.snapshotGenerationBySource,
        validFeedIds,
        validCategoryIds,
      ),
      changedMembershipKeys,
    ),
    pendingSnapshotGenerationBySource: withoutSourceKeys(
      pruneSourceRecord(
        state.pendingSnapshotGenerationBySource,
        validFeedIds,
        validCategoryIds,
      ),
      changedMembershipKeys,
    ),
    selectedSource: selectedSourceMissing
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedSourceMissing ? null : state.selectedEntryId,
    feedSearchQuery: selectedSourceMissing ? "" : state.feedSearchQuery,
    paneStatus: {
      ...state.paneStatus,
      subscriptions: "ready",
      queue:
        selectedSourceMissing || selectedMembershipChanged
          ? "idle"
          : state.paneStatus.queue,
    },
    requestActivity: { ...state.requestActivity, subscriptions: false },
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
    paneStatus: {
      ...state.paneStatus,
      subscriptions:
        state.paneStatus.subscriptions === "ready" ? "ready" : "error",
    },
    requestActivity: { ...state.requestActivity, subscriptions: false },
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
  const queueBySourceKey = feedChanged && previous
    ? withoutFeedEntries(state.queueBySourceKey, state, previous.feedId)
    : state.queueBySourceKey
  const pendingNewEntriesBySource = feedChanged && previous
    ? withoutFeedEntries(state.pendingNewEntriesBySource, state, previous.feedId)
    : state.pendingNewEntriesBySource
  const subscriptionsById = {
    ...state.subscriptionsById,
    [subscription.subscriptionId]: reconciled,
  }
  const changedMembershipKeys = membershipChangedSourceKeys(
    Object.values(state.subscriptionsById),
    Object.values(subscriptionsById),
  )
  const selectedMembershipChanged = changedMembershipKeys.has(
    sourceKey(state.selectedSource),
  )
  const nextQueueBySourceKey = withoutSourceKeys(
    withoutSourceKey(queueBySourceKey, feedChanged ? previousFeedKey : null),
    changedMembershipKeys,
  )
  const nextPendingNewEntriesBySource = withoutSourceKeys(
    withoutSourceKey(
      pendingNewEntriesBySource,
      feedChanged ? previousFeedKey : null,
    ),
    changedMembershipKeys,
  )
  return {
    ...state,
    subscriptionsById,
    subscriptionOrder: exists
      ? state.subscriptionOrder
      : [...state.subscriptionOrder, subscription.subscriptionId],
    retiredFeedIds,
    queueBySourceKey: nextQueueBySourceKey,
    nextCursorBySourceKey: withoutSourceKeys(
      withoutSourceKey(
        state.nextCursorBySourceKey,
        feedChanged ? previousFeedKey : null,
      ),
      changedMembershipKeys,
    ),
    pendingNewEntriesBySource: nextPendingNewEntriesBySource,
    pendingNewEntryCountBySource: countsForQueues(nextPendingNewEntriesBySource),
    snapshotGenerationBySource: withoutSourceKeys(
      withoutSourceKey(
        state.snapshotGenerationBySource,
        feedChanged ? previousFeedKey : null,
      ),
      changedMembershipKeys,
    ),
    pendingSnapshotGenerationBySource: withoutSourceKeys(
      withoutSourceKey(
        state.pendingSnapshotGenerationBySource,
        feedChanged ? previousFeedKey : null,
      ),
      changedMembershipKeys,
    ),
    selectedSource: selectedPreviousFeed
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedPreviousFeed ? null : state.selectedEntryId,
    feedSearchQuery: selectedPreviousFeed ? "" : state.feedSearchQuery,
    paneStatus: {
      ...state.paneStatus,
      queue:
        selectedPreviousFeed || selectedMembershipChanged
          ? "idle"
          : state.paneStatus.queue,
    },
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
  const queueBySourceKey = withoutFeedEntries(
    state.queueBySourceKey,
    state,
    subscription.feedId,
  )
  const pendingNewEntriesBySource = withoutFeedEntries(
    state.pendingNewEntriesBySource,
    state,
    subscription.feedId,
  )
  const snapshotGenerationBySource = { ...state.snapshotGenerationBySource }
  const nextCursorBySourceKey = { ...state.nextCursorBySourceKey }
  const pendingSnapshotGenerationBySource = {
    ...state.pendingSnapshotGenerationBySource,
  }
  delete queueBySourceKey[feedKey]
  delete pendingNewEntriesBySource[feedKey]
  delete snapshotGenerationBySource[feedKey]
  delete nextCursorBySourceKey[feedKey]
  delete pendingSnapshotGenerationBySource[feedKey]
  const changedMembershipKeys = membershipChangedSourceKeys(
    Object.values(state.subscriptionsById),
    Object.values(subscriptionsById),
  )
  const selectedMembershipChanged = changedMembershipKeys.has(
    sourceKey(state.selectedSource),
  )
  const nextQueueBySourceKey = withoutSourceKeys(
    queueBySourceKey,
    changedMembershipKeys,
  )
  const nextPendingNewEntriesBySource = withoutSourceKeys(
    pendingNewEntriesBySource,
    changedMembershipKeys,
  )
  const selectedDeletedFeed =
    state.selectedSource.kind === "feed" &&
    state.selectedSource.feedId === subscription.feedId
  return {
    ...state,
    subscriptionsById,
    subscriptionOrder: state.subscriptionOrder.filter((id) => id !== subscriptionId),
    retiredFeedIds: { ...state.retiredFeedIds, [subscription.feedId]: true },
    queueBySourceKey: nextQueueBySourceKey,
    nextCursorBySourceKey: withoutSourceKeys(
      nextCursorBySourceKey,
      changedMembershipKeys,
    ),
    pendingNewEntriesBySource: nextPendingNewEntriesBySource,
    pendingNewEntryCountBySource: countsForQueues(nextPendingNewEntriesBySource),
    snapshotGenerationBySource: withoutSourceKeys(
      snapshotGenerationBySource,
      changedMembershipKeys,
    ),
    pendingSnapshotGenerationBySource: withoutSourceKeys(
      pendingSnapshotGenerationBySource,
      changedMembershipKeys,
    ),
    selectedSource: selectedDeletedFeed
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedDeletedFeed ? null : state.selectedEntryId,
    feedSearchQuery: selectedDeletedFeed ? "" : state.feedSearchQuery,
    paneStatus: {
      ...state.paneStatus,
      queue:
        selectedDeletedFeed || selectedMembershipChanged
          ? "idle"
          : state.paneStatus.queue,
    },
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

function pruneEntryQueues(
  values: Partial<Record<SourceKey, string[]>>,
  state: ReaderState,
  validFeedIds: Set<string>,
  validCategoryIds: Set<string>,
  feedIdsByCategory: Map<string, Set<string>>,
): Partial<Record<SourceKey, string[]>> {
  const next: Partial<Record<SourceKey, string[]>> = {}
  for (const [rawKey, entryIds] of Object.entries(values)) {
    const key = rawKey as SourceKey
    const visibleFeedIds = visibleFeedIdsForSourceKey(
      key,
      validFeedIds,
      validCategoryIds,
      feedIdsByCategory,
    )
    if (!visibleFeedIds) continue
    next[key] = (entryIds ?? []).filter((entryId) => {
      const entry = state.entriesById[entryId]
      return entry !== undefined && visibleFeedIds.has(entry.feedId)
    })
  }
  return next
}

function withoutFeedEntries(
  values: Partial<Record<SourceKey, string[]>>,
  state: ReaderState,
  feedId: string,
): Partial<Record<SourceKey, string[]>> {
  return Object.fromEntries(
    Object.entries(values).map(([key, entryIds]) => [
      key,
      (entryIds ?? []).filter((entryId) => {
        const entry = state.entriesById[entryId]
        return entry !== undefined && entry.feedId !== feedId
      }),
    ]),
  )
}

function countsForQueues(
  values: Partial<Record<SourceKey, string[]>>,
): Partial<Record<SourceKey, number>> {
  return Object.fromEntries(
    Object.entries(values).map(([key, entryIds]) => [key, entryIds?.length ?? 0]),
  )
}

function pruneSourceRecord<T>(
  values: Partial<Record<SourceKey, T>>,
  validFeedIds: Set<string>,
  validCategoryIds: Set<string>,
): Partial<Record<SourceKey, T>> {
  return Object.fromEntries(
    Object.entries(values).filter(([key]) =>
      key.startsWith("feed:")
        ? validFeedIds.has(key.slice("feed:".length))
        : !key.startsWith("category:") ||
          validCategoryIds.has(key.slice("category:".length)),
    ),
  )
}

function visibleFeedIdsForSourceKey(
  key: SourceKey,
  validFeedIds: Set<string>,
  validCategoryIds: Set<string>,
  feedIdsByCategory: Map<string, Set<string>>,
): Set<string> | null {
  if (key.startsWith("feed:")) {
    const feedId = key.slice("feed:".length)
    return validFeedIds.has(feedId) ? new Set([feedId]) : null
  }
  if (key.startsWith("category:")) {
    const categoryId = key.slice("category:".length)
    if (!validCategoryIds.has(categoryId)) return null
    return feedIdsByCategory.get(categoryId) ?? new Set()
  }
  return validFeedIds
}

function membershipChangedSourceKeys(
  previous: Array<ReaderState["subscriptionsById"][string]>,
  next: Array<ReaderState["subscriptionsById"][string]>,
): Set<SourceKey> {
  const changed = new Set<SourceKey>()
  const previousFeeds = new Set(previous.map((subscription) => subscription.feedId))
  const nextFeeds = new Set(next.map((subscription) => subscription.feedId))
  if (!sameStringSet(previousFeeds, nextFeeds)) {
    changed.add("smart:UNREAD")
    changed.add("smart:ALL")
    changed.add("smart:STARRED")
  }

  const previousByCategory = feedIdsByCategory(previous)
  const nextByCategory = feedIdsByCategory(next)
  for (const categoryId of new Set([
    ...previousByCategory.keys(),
    ...nextByCategory.keys(),
  ])) {
    if (!sameStringSet(
      previousByCategory.get(categoryId) ?? new Set(),
      nextByCategory.get(categoryId) ?? new Set(),
    )) {
      changed.add(sourceKey({ kind: "category", categoryId }))
    }
  }
  return changed
}

function feedIdsByCategory(
  subscriptions: Array<ReaderState["subscriptionsById"][string]>,
): Map<string, Set<string>> {
  const result = new Map<string, Set<string>>()
  for (const subscription of subscriptions) {
    if (subscription.categoryId === null) continue
    const feeds = result.get(subscription.categoryId) ?? new Set<string>()
    feeds.add(subscription.feedId)
    result.set(subscription.categoryId, feeds)
  }
  return result
}

function sameStringSet(left: Set<string>, right: Set<string>): boolean {
  return left.size === right.size && [...left].every((value) => right.has(value))
}

function withoutSourceKeys<T>(
  values: Partial<Record<SourceKey, T>>,
  keys: Set<SourceKey>,
): Partial<Record<SourceKey, T>> {
  if (keys.size === 0) return values
  const next = { ...values }
  for (const key of keys) delete next[key]
  return next
}
