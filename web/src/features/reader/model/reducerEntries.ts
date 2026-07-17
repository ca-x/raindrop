import type {
  EntryDetailResponse,
  EntryListItemResponse,
} from "../api/reader.generated"
import { entryDetailToListItem, updateDetailsFromEntries } from "./reducerDetails"
import { reconcileDetail, reconcileListEntry } from "./reducerOptimistic"
import { sourceKey, type ReaderSource, type ReaderState } from "./types"

interface SourceReceipt {
  source: ReaderSource
  generation: number
  entries: EntryListItemResponse[]
  mode: "replace" | "discover"
}

interface DetailReceipt {
  entryId: string
  generation: number
  detail: EntryDetailResponse
}

export function receiveSource(state: ReaderState, receipt: SourceReceipt): ReaderState {
  if (receipt.generation !== state.requestGenerationByPane.queue) return state
  if (sourceKey(receipt.source) !== sourceKey(state.selectedSource)) return state
  const entries = receipt.entries.map((entry) => reconcileListEntry(state, entry))
  const entriesById = { ...state.entriesById }
  for (const entry of entries) entriesById[entry.entryId] = entry
  const key = sourceKey(receipt.source)
  const receivedIds = entries.map((entry) => entry.entryId)
  const currentQueue = state.queueBySourceKey[key] ?? []
  const pendingIds = state.pendingNewEntriesBySource[key] ?? []
  const discoveredIds = receivedIds.filter(
    (entryId) => !currentQueue.includes(entryId) && !pendingIds.includes(entryId),
  )
  return {
    ...state,
    entriesById,
    detailsById: updateDetailsFromEntries(state.detailsById, entries),
    queueBySourceKey: {
      ...state.queueBySourceKey,
      [key]: receipt.mode === "replace" ? receivedIds : currentQueue,
    },
    pendingNewEntriesBySource: {
      ...state.pendingNewEntriesBySource,
      [key]: receipt.mode === "replace" ? [] : [...pendingIds, ...discoveredIds],
    },
    pendingNewEntryCountBySource: {
      ...state.pendingNewEntryCountBySource,
      [key]: receipt.mode === "replace" ? 0 : pendingIds.length + discoveredIds.length,
    },
    paneStatus: { ...state.paneStatus, queue: "ready" },
  }
}

export function receiveDetail(state: ReaderState, receipt: DetailReceipt): ReaderState {
  if (
    receipt.generation !== state.requestGenerationByPane.detail ||
    receipt.entryId !== state.selectedEntryId
  ) {
    return state
  }
  const detail = reconcileDetail(state, receipt.detail)
  return {
    ...state,
    detailsById: { ...state.detailsById, [receipt.entryId]: detail },
    entriesById: {
      ...state.entriesById,
      [receipt.entryId]: entryDetailToListItem(detail),
    },
    paneStatus: { ...state.paneStatus, detail: "ready" },
    errors: { ...state.errors, detail: null },
  }
}
