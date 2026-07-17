import type {
  EntryDetailResponse,
  EntryListItemResponse,
  EntryListState,
} from "../api/reader.generated"
import type { Subscription } from "../api/subscription.generated"

export type ReaderSource =
  | { kind: "smart"; state: EntryListState }
  | { kind: "feed"; feedId: string }

export type SourceKey = `smart:${EntryListState}` | `feed:${string}`

export type EntryMutationField = "isRead" | "isStarred"

export interface OptimisticMutationSnapshot {
  entryId: string
  field: EntryMutationField
  entryValue?: boolean
  detailValue?: boolean
  subscriptionId?: string
  unreadDelta: number
}

export type PaneStatus = "idle" | "loading" | "ready" | "error"

export interface ReaderState {
  subscriptionsById: Record<string, Subscription>
  subscriptionOrder: string[]
  entriesById: Record<string, EntryListItemResponse>
  queueBySourceKey: Partial<Record<SourceKey, string[]>>
  detailsById: Record<string, EntryDetailResponse>
  selectedSource: ReaderSource
  selectedEntryId: string | null
  requestGenerationByPane: {
    subscriptions: number
    queue: number
    detail: number
  }
  pendingNewEntriesBySource: Partial<Record<SourceKey, string[]>>
  pendingNewEntryCountBySource: Partial<Record<SourceKey, number>>
  scrollAnchorByRoute: Record<string, number>
  paneStatus: {
    subscriptions: PaneStatus
    queue: PaneStatus
    detail: PaneStatus
  }
  errors: {
    subscriptions: string | null
    queue: string | null
    detail: string | null
    mutation: string | null
  }
  pendingMutationByEntryId: Record<
    string,
    Partial<Record<EntryMutationField, number>>
  >
  optimisticMutationsById: Record<number, OptimisticMutationSnapshot>
}

export function sourceKey(source: ReaderSource): SourceKey {
  return source.kind === "feed" ? `feed:${source.feedId}` : `smart:${source.state}`
}
