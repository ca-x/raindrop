import type { ReaderCacheSnapshot } from "../cache/readerCache"
import { sourceKey, type ReaderState } from "./types"

export function hydrateReaderCacheState(
  state: ReaderState,
  cached: ReaderCacheSnapshot,
): ReaderState {
  // Cache hydration can finish after the network request has already returned.
  // Never let an older snapshot overwrite an authoritative (or failed) request
  // result; only fill panes that are still cold or have no usable response.
  const organizationResponseSettled =
    !state.requestActivity.subscriptions &&
    state.paneStatus.subscriptions !== "idle"
  const hasOrganizationProjection =
    state.subscriptionOrder.length > 0 || state.categoryOrder.length > 0
  const shouldHydrateOrganization =
    (!organizationResponseSettled && !hasOrganizationProjection) ||
    state.errors.subscriptions !== null
  const subscriptions = cached.subscriptions
  const categoriesById = shouldHydrateOrganization
    ? Object.fromEntries(
        cached.categories.map((category) => [category.categoryId, category]),
      )
    : state.categoriesById
  const subscriptionsById = shouldHydrateOrganization
    ? Object.fromEntries(
        subscriptions.map((subscription) => [subscription.subscriptionId, subscription]),
      )
    : state.subscriptionsById
  const matchingSource = sourceKey(cached.source) === sourceKey(state.selectedSource)
  const hasCachedQueue = matchingSource && cached.snapshotGeneration !== null
  const queueResponseSettled =
    !state.requestActivity.queue && state.paneStatus.queue !== "idle"
  const shouldHydrateQueue =
    hasCachedQueue && (!queueResponseSettled || state.errors.queue !== null)

  if (!shouldHydrateOrganization && !shouldHydrateQueue) {
    return {
      ...state,
      scrollAnchorByRoute: {
        ...state.scrollAnchorByRoute,
        ...cached.scrollAnchorByRoute,
      },
    }
  }

  if (!shouldHydrateQueue) {
    return {
      ...state,
      categoriesById,
      categoryOrder: shouldHydrateOrganization
        ? cached.categories.map((category) => category.categoryId)
        : state.categoryOrder,
      subscriptionsById,
      subscriptionOrder: shouldHydrateOrganization
        ? subscriptions.map((subscription) => subscription.subscriptionId)
        : state.subscriptionOrder,
      subscriptionsAuthoritative: shouldHydrateOrganization
        ? false
        : state.subscriptionsAuthoritative,
      scrollAnchorByRoute: {
        ...state.scrollAnchorByRoute,
        ...cached.scrollAnchorByRoute,
      },
      paneStatus: {
        ...state.paneStatus,
        subscriptions: shouldHydrateOrganization
          ? "ready"
          : state.paneStatus.subscriptions,
      },
      errors: {
        ...state.errors,
        subscriptions: shouldHydrateOrganization ? null : state.errors.subscriptions,
      },
    }
  }

  const key = sourceKey(state.selectedSource)
  return {
    ...state,
    categoriesById,
    categoryOrder: shouldHydrateOrganization
      ? cached.categories.map((category) => category.categoryId)
      : state.categoryOrder,
    subscriptionsById,
    subscriptionOrder: shouldHydrateOrganization
      ? subscriptions.map((subscription) => subscription.subscriptionId)
      : state.subscriptionOrder,
    subscriptionsAuthoritative: shouldHydrateOrganization
      ? false
      : state.subscriptionsAuthoritative,
    entriesById: {
      ...state.entriesById,
      ...(shouldHydrateQueue
        ? Object.fromEntries(cached.entries.map((entry) => [
            entry.entryId,
            { ...entry, siteUrl: null, canonicalUrl: null },
          ]))
        : {}),
    },
    queueBySourceKey: shouldHydrateQueue
      ? { ...state.queueBySourceKey, [key]: [...cached.queue] }
      : state.queueBySourceKey,
    snapshotGenerationBySource: shouldHydrateQueue
      ? {
          ...state.snapshotGenerationBySource,
          [key]: cached.snapshotGeneration,
        }
      : state.snapshotGenerationBySource,
    scrollAnchorByRoute: {
      ...state.scrollAnchorByRoute,
      ...cached.scrollAnchorByRoute,
    },
    paneStatus: {
      ...state.paneStatus,
      subscriptions: shouldHydrateOrganization
        ? "ready"
        : state.paneStatus.subscriptions,
      queue: shouldHydrateQueue ? "ready" : state.paneStatus.queue,
    },
    errors: {
      ...state.errors,
      subscriptions: shouldHydrateOrganization ? null : state.errors.subscriptions,
      queue: shouldHydrateQueue ? null : state.errors.queue,
    },
  }
}
