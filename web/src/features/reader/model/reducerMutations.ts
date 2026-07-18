import type { EntryStateResponse } from "../api/reader.generated"
import type {
  EntryMutationField,
  OptimisticMutationSnapshot,
  ReaderState,
} from "./types"

interface MutationStart {
  mutationId: number
  entryId: string
  field: EntryMutationField
  value: boolean
}

export function startEntryMutation(
  state: ReaderState,
  action: MutationStart,
): ReaderState {
  const entry = state.entriesById[action.entryId]
  const detail = state.detailsById[action.entryId]
  const currentValue = entry?.[action.field] ?? detail?.[action.field]
  if (currentValue === undefined) return state

  const subscriptionId = findSubscriptionId(state, entry?.feedId ?? detail?.feedId)
  const unreadDelta =
    action.field === "isRead" && currentValue !== action.value
      ? action.value
        ? -1
        : 1
      : 0
  const snapshot: OptimisticMutationSnapshot = {
    entryId: action.entryId,
    field: action.field,
    entryValue: entry?.[action.field],
    detailValue: detail?.[action.field],
    subscriptionId,
    unreadDelta,
  }

  return {
    ...applyMutationValue(state, snapshot, action.value),
    errors: { ...state.errors, mutation: null },
    pendingMutationByEntryId: {
      ...state.pendingMutationByEntryId,
      [action.entryId]: {
        ...state.pendingMutationByEntryId[action.entryId],
        [action.field]: action.mutationId,
      },
    },
    optimisticMutationsById: {
      ...state.optimisticMutationsById,
      [action.mutationId]: snapshot,
    },
  }
}

export function failEntryMutation(
  state: ReaderState,
  mutationId: number,
  error: string,
): ReaderState {
  const snapshot = state.optimisticMutationsById[mutationId]
  if (!snapshot) return state
  const isLatest =
    state.pendingMutationByEntryId[snapshot.entryId]?.[snapshot.field] === mutationId
  const withoutSnapshot = removeSnapshot(state, mutationId)
  if (!isLatest) return withoutSnapshot

  const entry = withoutSnapshot.entriesById[snapshot.entryId]
  const detail = withoutSnapshot.detailsById[snapshot.entryId]
  return {
    ...withoutSnapshot,
    entriesById:
      entry && snapshot.entryValue !== undefined
        ? {
            ...withoutSnapshot.entriesById,
            [snapshot.entryId]: {
              ...entry,
              [snapshot.field]: snapshot.entryValue,
            },
          }
        : withoutSnapshot.entriesById,
    detailsById:
      detail && snapshot.detailValue !== undefined
        ? {
            ...withoutSnapshot.detailsById,
            [snapshot.entryId]: {
              ...detail,
              [snapshot.field]: snapshot.detailValue,
            },
          }
        : withoutSnapshot.detailsById,
    subscriptionsById: undoUnreadDelta(withoutSnapshot, snapshot),
    errors: { ...withoutSnapshot.errors, mutation: error },
  }
}

export function succeedEntryMutation(
  state: ReaderState,
  mutationId: number,
  response: EntryStateResponse,
): ReaderState {
  const snapshot = state.optimisticMutationsById[mutationId]
  if (!snapshot) return state
  const isLatest =
    state.pendingMutationByEntryId[snapshot.entryId]?.[snapshot.field] === mutationId
  let next = removeSnapshot(state, mutationId)
  if (!isLatest || response.entryId !== snapshot.entryId) return next

  next = applyAuthoritativeValue(next, snapshot.entryId, snapshot.field, response[snapshot.field])
  const otherField: EntryMutationField =
    snapshot.field === "isRead" ? "isStarred" : "isRead"
  if (next.pendingMutationByEntryId[snapshot.entryId]?.[otherField] === undefined) {
    next = applyAuthoritativeValue(next, snapshot.entryId, otherField, response[otherField])
  }
  return next
}

function applyAuthoritativeValue(
  state: ReaderState,
  entryId: string,
  field: EntryMutationField,
  value: boolean,
): ReaderState {
  const entry = state.entriesById[entryId]
  const detail = state.detailsById[entryId]
  const currentValue = entry?.[field] ?? detail?.[field]
  if (currentValue === undefined || currentValue === value) return state
  const subscriptionId = findSubscriptionId(state, entry?.feedId ?? detail?.feedId)
  return applyMutationValue(
    state,
    {
      entryId,
      field,
      entryValue: entry?.[field],
      detailValue: detail?.[field],
      subscriptionId,
      unreadDelta: field === "isRead" ? (value ? -1 : 1) : 0,
    },
    value,
  )
}

function applyMutationValue(
  state: ReaderState,
  snapshot: OptimisticMutationSnapshot,
  value: boolean,
): ReaderState {
  const entry = state.entriesById[snapshot.entryId]
  const detail = state.detailsById[snapshot.entryId]
  const subscription = snapshot.subscriptionId
    ? state.subscriptionsById[snapshot.subscriptionId]
    : undefined
  return {
    ...state,
    entriesById: entry
      ? {
          ...state.entriesById,
          [snapshot.entryId]: { ...entry, [snapshot.field]: value },
        }
      : state.entriesById,
    detailsById: detail
      ? {
          ...state.detailsById,
          [snapshot.entryId]: { ...detail, [snapshot.field]: value },
        }
      : state.detailsById,
    subscriptionsById:
      subscription && snapshot.unreadDelta !== 0
        ? {
            ...state.subscriptionsById,
            [subscription.subscriptionId]: {
              ...subscription,
              unreadCount: Math.max(0, subscription.unreadCount + snapshot.unreadDelta),
            },
          }
        : state.subscriptionsById,
  }
}

function undoUnreadDelta(
  state: ReaderState,
  snapshot: OptimisticMutationSnapshot,
): ReaderState["subscriptionsById"] {
  const subscription = snapshot.subscriptionId
    ? state.subscriptionsById[snapshot.subscriptionId]
    : undefined
  if (!subscription || snapshot.unreadDelta === 0) return state.subscriptionsById
  return {
    ...state.subscriptionsById,
    [subscription.subscriptionId]: {
      ...subscription,
      unreadCount: Math.max(0, subscription.unreadCount - snapshot.unreadDelta),
    },
  }
}

function removeSnapshot(state: ReaderState, mutationId: number): ReaderState {
  const snapshot = state.optimisticMutationsById[mutationId]
  if (!snapshot) return state
  const optimisticMutationsById = { ...state.optimisticMutationsById }
  delete optimisticMutationsById[mutationId]
  const pendingForEntry = { ...state.pendingMutationByEntryId[snapshot.entryId] }
  if (pendingForEntry[snapshot.field] === mutationId) delete pendingForEntry[snapshot.field]
  return {
    ...state,
    optimisticMutationsById,
    pendingMutationByEntryId: {
      ...state.pendingMutationByEntryId,
      [snapshot.entryId]: pendingForEntry,
    },
  }
}

function findSubscriptionId(
  state: ReaderState,
  feedId: string | undefined,
): string | undefined {
  if (!feedId) return undefined
  return state.subscriptionOrder.find(
    (subscriptionId) => state.subscriptionsById[subscriptionId]?.feedId === feedId,
  )
}
