import type {
  EntryDetailResponse,
  EntryListItemResponse,
  EntryStateResponse,
} from "../api/reader.generated"
import type { Category } from "../api/organization.generated"
import type {
  Refresh,
  Subscription,
} from "../api/subscription.generated"
import {
  failEntryMutation,
  startEntryMutation,
  succeedEntryMutation,
} from "./reducerMutations"
import { deleteCategoryState, upsertCategory } from "./reducerCategories"
import { receiveDetail, receiveSource } from "./reducerEntries"
import {
  deleteSubscriptionState,
  failSubscriptions,
  receiveSubscriptions,
  requestSubscriptions,
  updateSubscriptionRefresh,
  upsertSubscription,
} from "./reducerSubscriptions"
import { sourceKey, type ReaderSource, type ReaderState, type SourceKey } from "./types"
import type { EntryMutationField } from "./types"

export type ReaderAction =
  | { type: "subscriptionsRequested"; generation: number }
  | {
      type: "subscriptionsReceived"
      generation: number
      subscriptions: Subscription[]
      categories: Category[]
    }
  | { type: "subscriptionsFailed"; generation: number; error: string }
  | { type: "subscriptionUpserted"; subscription: Subscription }
  | { type: "subscriptionDeleted"; subscriptionId: string }
  | { type: "subscriptionRefreshUpdated"; subscriptionId: string; refresh: Refresh }
  | { type: "categoryUpserted"; category: Category }
  | { type: "categoryDeleted"; categoryId: string }
  | { type: "sourceSelected"; source: ReaderSource }
  | { type: "feedSearchChanged"; query: string }
  | { type: "entrySelected"; entryId: string | null }
  | { type: "scrollAnchorRecorded"; route: string; offset: number }
  | { type: "sourceRequested"; source: ReaderSource; generation: number }
  | {
      type: "sourceReceived"
      source: ReaderSource
      generation: number
      entries: EntryListItemResponse[]
      snapshotGeneration: number
      mode: "replace" | "discover"
    }
  | { type: "sourceFailed"; source: ReaderSource; generation: number; error: string }
  | { type: "pendingEntriesMerged"; source: ReaderSource }
  | { type: "detailRequested"; entryId: string; generation: number }
  | {
      type: "detailReceived"
      entryId: string
      generation: number
      detail: EntryDetailResponse
    }
  | { type: "detailFailed"; entryId: string; generation: number; error: string }
  | {
      type: "entryMutationStarted"
      mutationId: number
      entryId: string
      field: EntryMutationField
      value: boolean
    }
  | { type: "entryMutationSucceeded"; mutationId: number; state: EntryStateResponse }
  | { type: "entryMutationFailed"; mutationId: number; error: string }
  | { type: "mutationErrorSet"; error: string }
  | { type: "mutationErrorCleared" }
  | { type: "sessionExpired" }

export const initialReaderState: ReaderState = {
  categoriesById: {},
  categoryOrder: [],
  subscriptionsById: {},
  subscriptionOrder: [],
  entriesById: {},
  queueBySourceKey: {},
  detailsById: {},
  selectedSource: { kind: "smart", state: "UNREAD" },
  selectedEntryId: null,
  requestGenerationByPane: { subscriptions: 0, queue: 0, detail: 0 },
  pendingNewEntriesBySource: {},
  pendingNewEntryCountBySource: {},
  snapshotGenerationBySource: {},
  pendingSnapshotGenerationBySource: {},
  feedSearchQuery: "",
  scrollAnchorByRoute: {},
  paneStatus: { subscriptions: "idle", queue: "idle", detail: "idle" },
  errors: { subscriptions: null, queue: null, detail: null, mutation: null },
  pendingMutationByEntryId: {},
  optimisticMutationsById: {},
}

export function readerReducer(state: ReaderState, action: ReaderAction): ReaderState {
  switch (action.type) {
    case "subscriptionsRequested":
      return requestSubscriptions(state, action.generation)
    case "subscriptionsReceived":
      return receiveSubscriptions(
        state,
        action.generation,
        action.subscriptions,
        action.categories,
      )
    case "subscriptionsFailed":
      return failSubscriptions(state, action.generation, action.error)
    case "subscriptionUpserted":
      return upsertSubscription(state, action.subscription)
    case "subscriptionDeleted":
      return deleteSubscriptionState(state, action.subscriptionId)
    case "subscriptionRefreshUpdated":
      return updateSubscriptionRefresh(state, action.subscriptionId, action.refresh)
    case "categoryUpserted":
      return upsertCategory(state, action.category)
    case "categoryDeleted":
      return deleteCategoryState(state, action.categoryId)
    case "sourceSelected":
      return {
        ...state,
        selectedSource: action.source,
        selectedEntryId: null,
        feedSearchQuery: "",
      }
    case "feedSearchChanged":
      return { ...state, feedSearchQuery: action.query, selectedEntryId: null }
    case "entrySelected":
      return { ...state, selectedEntryId: action.entryId }
    case "scrollAnchorRecorded":
      return {
        ...state,
        scrollAnchorByRoute: {
          ...state.scrollAnchorByRoute,
          [action.route]: action.offset,
        },
      }
    case "sourceRequested":
      return {
        ...state,
        requestGenerationByPane: {
          ...state.requestGenerationByPane,
          queue: action.generation,
        },
        paneStatus: { ...state.paneStatus, queue: "loading" },
        errors: { ...state.errors, queue: null },
      }
    case "sourceReceived": {
      return receiveSource(state, action)
    }
    case "sourceFailed":
      if (
        action.generation !== state.requestGenerationByPane.queue ||
        sourceKey(action.source) !== sourceKey(state.selectedSource)
      ) {
        return state
      }
      return {
        ...state,
        paneStatus: { ...state.paneStatus, queue: "error" },
        errors: { ...state.errors, queue: action.error },
      }
    case "pendingEntriesMerged": {
      const key = sourceKey(action.source)
      const pendingIds = state.pendingNewEntriesBySource[key] ?? []
      const currentQueue = state.queueBySourceKey[key] ?? []
      return {
        ...state,
        queueBySourceKey: {
          ...state.queueBySourceKey,
          [key]: [...pendingIds, ...currentQueue],
        },
        pendingNewEntriesBySource: {
          ...state.pendingNewEntriesBySource,
          [key]: [],
        },
        pendingNewEntryCountBySource: {
          ...state.pendingNewEntryCountBySource,
          [key]: 0,
        },
        snapshotGenerationBySource: {
          ...state.snapshotGenerationBySource,
          [key]:
            state.pendingSnapshotGenerationBySource[key] ??
            state.snapshotGenerationBySource[key] ??
            0,
        },
        pendingSnapshotGenerationBySource: withoutKey(
          state.pendingSnapshotGenerationBySource,
          key,
        ),
      }
    }
    case "detailRequested":
      return {
        ...state,
        requestGenerationByPane: {
          ...state.requestGenerationByPane,
          detail: action.generation,
        },
        paneStatus: { ...state.paneStatus, detail: "loading" },
        errors: { ...state.errors, detail: null },
      }
    case "detailReceived":
      return receiveDetail(state, action)
    case "detailFailed":
      if (
        action.generation !== state.requestGenerationByPane.detail ||
        action.entryId !== state.selectedEntryId
      ) {
        return state
      }
      return {
        ...state,
        paneStatus: { ...state.paneStatus, detail: "error" },
        errors: { ...state.errors, detail: action.error },
      }
    case "entryMutationStarted":
      return startEntryMutation(state, action)
    case "entryMutationSucceeded":
      return succeedEntryMutation(state, action.mutationId, action.state)
    case "entryMutationFailed":
      return failEntryMutation(state, action.mutationId, action.error)
    case "mutationErrorSet":
      return { ...state, errors: { ...state.errors, mutation: action.error } }
    case "mutationErrorCleared":
      return { ...state, errors: { ...state.errors, mutation: null } }
    case "sessionExpired":
      return {
        ...initialReaderState,
        requestGenerationByPane: state.requestGenerationByPane,
        paneStatus: { subscriptions: "idle", queue: "idle", detail: "idle" },
        errors: { subscriptions: null, queue: null, detail: null, mutation: null },
      }
  }
}

function withoutKey<T>(record: Partial<Record<SourceKey, T>>, key: SourceKey) {
  const next = { ...record }
  delete next[key]
  return next
}
