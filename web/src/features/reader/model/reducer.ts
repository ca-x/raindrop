import type {
  EntryDetailResponse,
  EntryListItemResponse,
  EntryStateResponse,
} from "../api/reader.generated"
import type {
  Refresh,
  Subscription,
} from "../api/subscription.generated"
import {
  failEntryMutation,
  startEntryMutation,
  succeedEntryMutation,
} from "./reducerMutations"
import {
  entryDetailToListItem,
  updateDetailsFromEntries,
} from "./reducerDetails"
import {
  deleteSubscriptionState,
  failSubscriptions,
  receiveSubscriptions,
  requestSubscriptions,
  updateSubscriptionRefresh,
  upsertSubscription,
} from "./reducerSubscriptions"
import { sourceKey, type ReaderSource, type ReaderState } from "./types"
import type { EntryMutationField } from "./types"

export type ReaderAction =
  | { type: "subscriptionsRequested"; generation: number }
  | {
      type: "subscriptionsReceived"
      generation: number
      subscriptions: Subscription[]
    }
  | { type: "subscriptionsFailed"; generation: number; error: string }
  | { type: "subscriptionUpserted"; subscription: Subscription }
  | { type: "subscriptionDeleted"; subscriptionId: string }
  | { type: "subscriptionRefreshUpdated"; subscriptionId: string; refresh: Refresh }
  | { type: "sourceSelected"; source: ReaderSource }
  | { type: "entrySelected"; entryId: string | null }
  | { type: "scrollAnchorRecorded"; route: string; offset: number }
  | { type: "sourceRequested"; source: ReaderSource; generation: number }
  | {
      type: "sourceReceived"
      source: ReaderSource
      generation: number
      entries: EntryListItemResponse[]
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
      return receiveSubscriptions(state, action.generation, action.subscriptions)
    case "subscriptionsFailed":
      return failSubscriptions(state, action.generation, action.error)
    case "subscriptionUpserted":
      return upsertSubscription(state, action.subscription)
    case "subscriptionDeleted":
      return deleteSubscriptionState(state, action.subscriptionId)
    case "subscriptionRefreshUpdated":
      return updateSubscriptionRefresh(state, action.subscriptionId, action.refresh)
    case "sourceSelected":
      return { ...state, selectedSource: action.source, selectedEntryId: null }
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
      if (action.generation !== state.requestGenerationByPane.queue) return state
      if (sourceKey(action.source) !== sourceKey(state.selectedSource)) return state
      const entriesById = { ...state.entriesById }
      for (const entry of action.entries) entriesById[entry.entryId] = entry
      const key = sourceKey(action.source)
      const receivedIds = action.entries.map((entry) => entry.entryId)
      const currentQueue = state.queueBySourceKey[key] ?? []
      const pendingIds = state.pendingNewEntriesBySource[key] ?? []
      const discoveredIds = receivedIds.filter(
        (entryId) => !currentQueue.includes(entryId) && !pendingIds.includes(entryId),
      )
      return {
        ...state,
        entriesById,
        detailsById: updateDetailsFromEntries(state.detailsById, action.entries),
        queueBySourceKey: {
          ...state.queueBySourceKey,
          [key]: action.mode === "replace" ? receivedIds : currentQueue,
        },
        pendingNewEntriesBySource: {
          ...state.pendingNewEntriesBySource,
          [key]: action.mode === "replace" ? [] : [...pendingIds, ...discoveredIds],
        },
        pendingNewEntryCountBySource: {
          ...state.pendingNewEntryCountBySource,
          [key]: action.mode === "replace" ? 0 : pendingIds.length + discoveredIds.length,
        },
        paneStatus: { ...state.paneStatus, queue: "ready" },
      }
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
      if (
        action.generation !== state.requestGenerationByPane.detail ||
        action.entryId !== state.selectedEntryId
      ) {
        return state
      }
      return {
        ...state,
        detailsById: { ...state.detailsById, [action.entryId]: action.detail },
        entriesById: {
          ...state.entriesById,
          [action.entryId]: entryDetailToListItem(action.detail),
        },
        paneStatus: { ...state.paneStatus, detail: "ready" },
        errors: { ...state.errors, detail: null },
      }
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
        requestGenerationByPane: { subscriptions: 0, queue: 0, detail: 0 },
        paneStatus: { subscriptions: "idle", queue: "idle", detail: "idle" },
        errors: { subscriptions: null, queue: null, detail: null, mutation: null },
      }
  }
}
