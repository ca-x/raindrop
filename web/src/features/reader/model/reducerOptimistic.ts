import type {
  EntryDetailResponse,
  EntryListItemResponse,
} from "../api/reader.generated"
import type { Subscription } from "../api/subscription.generated"
import type { ReaderState } from "./types"

type StatefulEntry = EntryListItemResponse | EntryDetailResponse

export function reconcileListEntry(
  state: ReaderState,
  entry: EntryListItemResponse,
): EntryListItemResponse {
  return reconcileEntryState(state, entry)
}

export function reconcileDetail(
  state: ReaderState,
  detail: EntryDetailResponse,
): EntryDetailResponse {
  return reconcileEntryState(state, detail)
}

export function reconcileSubscription(
  state: ReaderState,
  subscription: Subscription,
): Subscription {
  if (!hasPendingReadMutation(state, subscription.feedId)) return subscription
  const current = state.subscriptionsById[subscription.subscriptionId]
  return current ? { ...subscription, unreadCount: current.unreadCount } : subscription
}

function reconcileEntryState<T extends StatefulEntry>(state: ReaderState, incoming: T): T {
  const pending = state.pendingMutationByEntryId[incoming.entryId]
  if (!pending) return incoming
  const current = state.entriesById[incoming.entryId] ?? state.detailsById[incoming.entryId]
  if (!current) return incoming
  return {
    ...incoming,
    isRead: pending.isRead === undefined ? incoming.isRead : current.isRead,
    isStarred: pending.isStarred === undefined ? incoming.isStarred : current.isStarred,
  }
}

function hasPendingReadMutation(state: ReaderState, feedId: string): boolean {
  return Object.entries(state.pendingMutationByEntryId).some(([entryId, pending]) => {
    if (pending.isRead === undefined) return false
    const entry = state.entriesById[entryId] ?? state.detailsById[entryId]
    return entry?.feedId === feedId
  })
}
