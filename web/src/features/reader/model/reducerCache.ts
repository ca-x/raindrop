import type { ReaderCacheSnapshot } from "../cache/readerCache"
import { sourceKey, type ReaderState } from "./types"

export function hydrateReaderCacheState(
  state: ReaderState,
  cached: ReaderCacheSnapshot,
): ReaderState {
  const categoriesById = Object.fromEntries(
    cached.categories.map((category) => [category.categoryId, category]),
  )
  const subscriptionsById = Object.fromEntries(
    cached.subscriptions.map((subscription) => [subscription.subscriptionId, subscription]),
  )
  const matchingSource = sourceKey(cached.source) === sourceKey(state.selectedSource)
  const hasQueue = matchingSource && cached.snapshotGeneration !== null

  if (!hasQueue) {
    return {
      ...state,
      categoriesById,
      categoryOrder: cached.categories.map((category) => category.categoryId),
      subscriptionsById,
      subscriptionOrder: cached.subscriptions.map(
        (subscription) => subscription.subscriptionId,
      ),
      scrollAnchorByRoute: {
        ...state.scrollAnchorByRoute,
        ...cached.scrollAnchorByRoute,
      },
      paneStatus: { ...state.paneStatus, subscriptions: "ready" },
      errors: { ...state.errors, subscriptions: null },
    }
  }

  const key = sourceKey(state.selectedSource)
  return {
    ...state,
    categoriesById,
    categoryOrder: cached.categories.map((category) => category.categoryId),
    subscriptionsById,
    subscriptionOrder: cached.subscriptions.map(
      (subscription) => subscription.subscriptionId,
    ),
    entriesById: {
      ...state.entriesById,
      ...Object.fromEntries(cached.entries.map((entry) => [entry.entryId, entry])),
    },
    queueBySourceKey: { ...state.queueBySourceKey, [key]: [...cached.queue] },
    snapshotGenerationBySource: {
      ...state.snapshotGenerationBySource,
      [key]: cached.snapshotGeneration,
    },
    scrollAnchorByRoute: {
      ...state.scrollAnchorByRoute,
      ...cached.scrollAnchorByRoute,
    },
    paneStatus: { ...state.paneStatus, subscriptions: "ready", queue: "ready" },
    errors: { ...state.errors, subscriptions: null, queue: null },
  }
}
