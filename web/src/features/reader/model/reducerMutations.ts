import type { EntryStateResponse } from "../api/reader.generated"
import {
  sourceKey,
  type EntryMutationField,
  type OptimisticMutationSnapshot,
  type ReaderState,
  type SourceKey,
} from "./types"

interface BulkReadCommit {
  entryIds: string[]
  affectedFeedIds: string[]
  retainedSourceKey: SourceKey
  retainedQueueEntryIds: string[] | null
  retainedPendingEntryIds: string[] | null
  retainedQueueGeneration: number
  retainedSnapshotGeneration: number | null
  retainedPendingSnapshotGeneration: number | null
  invalidateAllSources: boolean
}

interface MutationStart {
  mutationId: number
  entryId: string
  field: EntryMutationField
  value: boolean
}

export function commitBulkRead(
  state: ReaderState,
  action: BulkReadCommit,
): ReaderState {
  const affectedFeedIds = new Set(action.affectedFeedIds)
  const committedEntryIds = new Set(action.entryIds)
  const retainedSnapshotGeneration =
    state.snapshotGenerationBySource[action.retainedSourceKey] ?? null
  const retainedPendingSnapshotGeneration =
    state.pendingSnapshotGenerationBySource[action.retainedSourceKey] ?? null
  const projectionIsStable =
    sourceKey(state.selectedSource) === action.retainedSourceKey &&
    state.requestGenerationByPane.queue === action.retainedQueueGeneration &&
    retainedSnapshotGeneration === action.retainedSnapshotGeneration &&
    sameProjection(
      state.queueBySourceKey[action.retainedSourceKey],
      action.retainedQueueEntryIds,
    ) &&
    sameProjection(
      state.pendingNewEntriesBySource[action.retainedSourceKey],
      action.retainedPendingEntryIds,
    ) &&
    retainedPendingSnapshotGeneration ===
      action.retainedPendingSnapshotGeneration
  const retainedSourceKey = projectionIsStable ? action.retainedSourceKey : null
  const invalidatedSourceKeys = sourceKeysToInvalidate(
    state,
    affectedFeedIds,
    retainedSourceKey,
    action.invalidateAllSources,
  )
  let entriesById = state.entriesById
  let detailsById = state.detailsById

  for (const entryId of committedEntryIds) {
    const entry = entriesById[entryId]
    if (entry && !entry.isRead) {
      if (entriesById === state.entriesById) entriesById = { ...entriesById }
      entriesById[entryId] = { ...entry, isRead: true }
    }
    const detail = detailsById[entryId]
    if (detail && !detail.isRead) {
      if (detailsById === state.detailsById) detailsById = { ...detailsById }
      detailsById[entryId] = { ...detail, isRead: true }
    }
  }

  const {
    pendingMutationByEntryId,
    optimisticMutationsById,
  } = withoutEntryReadMutations(state, committedEntryIds)
  let queueBySourceKey = state.queueBySourceKey
  const unreadKey = "smart:UNREAD" as const
  const unreadQueue = queueBySourceKey[unreadKey]
  if (unreadQueue) {
    const remainingUnread = unreadQueue.filter(
      (entryId) => !committedEntryIds.has(entryId),
    )
    if (remainingUnread.length !== unreadQueue.length) {
      queueBySourceKey = {
        ...queueBySourceKey,
        [unreadKey]: remainingUnread,
      }
    }
  }
  queueBySourceKey = withoutSourceKeys(queueBySourceKey, invalidatedSourceKeys)

  let pendingNewEntriesBySource = state.pendingNewEntriesBySource
  for (const [key, entryIds] of Object.entries(pendingNewEntriesBySource)) {
    if (!entryIds) continue
    const remainingPending = entryIds.filter(
      (entryId) => !committedEntryIds.has(entryId),
    )
    if (remainingPending.length === entryIds.length) continue
    if (pendingNewEntriesBySource === state.pendingNewEntriesBySource) {
      pendingNewEntriesBySource = { ...pendingNewEntriesBySource }
    }
    pendingNewEntriesBySource[key as SourceKey] = remainingPending
  }
  pendingNewEntriesBySource = withoutSourceKeys(
    pendingNewEntriesBySource,
    invalidatedSourceKeys,
  )
  const pendingNewEntryCountBySource = Object.fromEntries(
    Object.entries(pendingNewEntriesBySource).map(([key, entryIds]) => [
      key,
      entryIds?.length ?? 0,
    ]),
  ) as ReaderState["pendingNewEntryCountBySource"]

  return {
    ...state,
    entriesById,
    detailsById,
    queueBySourceKey,
    nextCursorBySourceKey: withoutSourceKeys(
      state.nextCursorBySourceKey,
      invalidatedSourceKeys,
    ),
    pendingNewEntriesBySource,
    pendingNewEntryCountBySource,
    snapshotGenerationBySource: withoutSourceKeys(
      state.snapshotGenerationBySource,
      invalidatedSourceKeys,
    ),
    pendingSnapshotGenerationBySource: withoutSourceKeys(
      state.pendingSnapshotGenerationBySource,
      invalidatedSourceKeys,
    ),
    requestGenerationByPane: {
      ...state.requestGenerationByPane,
      subscriptions: state.requestGenerationByPane.subscriptions + 1,
      queue: state.requestGenerationByPane.queue + 1,
      detail: state.requestGenerationByPane.detail + 1,
    },
    paneStatus: invalidatedSourceKeys.has(sourceKey(state.selectedSource))
      ? { ...state.paneStatus, queue: "idle" }
      : state.paneStatus,
    errors: { ...state.errors, mutation: null },
    pendingMutationByEntryId,
    optimisticMutationsById,
  }
}

function sameProjection(
  current: string[] | undefined,
  retained: string[] | null,
): boolean {
  if (current === undefined || retained === null) {
    return current === undefined && retained === null
  }
  return (
    current.length === retained.length &&
    current.every((entryId, index) => entryId === retained[index])
  )
}

function withoutEntryReadMutations(
  state: ReaderState,
  entryIds: Set<string>,
): Pick<ReaderState, "pendingMutationByEntryId" | "optimisticMutationsById"> {
  let pendingMutationByEntryId = state.pendingMutationByEntryId
  for (const entryId of entryIds) {
    const pending = pendingMutationByEntryId[entryId]
    if (pending?.isRead === undefined) continue
    if (pendingMutationByEntryId === state.pendingMutationByEntryId) {
      pendingMutationByEntryId = { ...pendingMutationByEntryId }
    }
    const remaining = { ...pending }
    delete remaining.isRead
    if (Object.keys(remaining).length === 0) {
      delete pendingMutationByEntryId[entryId]
    } else {
      pendingMutationByEntryId[entryId] = remaining
    }
  }
  let optimisticMutationsById = state.optimisticMutationsById
  for (const [mutationId, snapshot] of Object.entries(optimisticMutationsById)) {
    if (snapshot.field !== "isRead" || !entryIds.has(snapshot.entryId)) continue
    if (optimisticMutationsById === state.optimisticMutationsById) {
      optimisticMutationsById = { ...optimisticMutationsById }
    }
    delete optimisticMutationsById[Number(mutationId)]
  }
  return { pendingMutationByEntryId, optimisticMutationsById }
}

function sourceKeysToInvalidate(
  state: ReaderState,
  affectedFeedIds: Set<string>,
  retainedSourceKey: SourceKey | null,
  invalidateAllSources: boolean,
): Set<SourceKey> {
  const knownKeys = new Set<SourceKey>()
  for (const record of [
    state.queueBySourceKey,
    state.nextCursorBySourceKey,
    state.pendingNewEntriesBySource,
    state.pendingNewEntryCountBySource,
    state.snapshotGenerationBySource,
    state.pendingSnapshotGenerationBySource,
  ]) {
    for (const key of Object.keys(record)) knownKeys.add(key as SourceKey)
  }
  const invalidated = new Set<SourceKey>()
  for (const key of knownKeys) {
    if (
      key !== retainedSourceKey &&
      (invalidateAllSources || sourceIntersectsFeeds(state, key, affectedFeedIds))
    ) {
      invalidated.add(key)
    }
  }
  return invalidated
}

function sourceIntersectsFeeds(
  state: ReaderState,
  key: SourceKey,
  affectedFeedIds: Set<string>,
): boolean {
  if (affectedFeedIds.size === 0) return false
  if (key.startsWith("smart:")) return true
  if (key.startsWith("feed:")) {
    return affectedFeedIds.has(key.slice("feed:".length))
  }
  const categoryId = key.slice("category:".length)
  return state.subscriptionOrder.some((subscriptionId) => {
    const subscription = state.subscriptionsById[subscriptionId]
    return Boolean(
      subscription &&
      subscription.categoryId === categoryId &&
      affectedFeedIds.has(subscription.feedId),
    )
  })
}

function withoutSourceKeys<T>(
  record: Partial<Record<SourceKey, T>>,
  keys: Set<SourceKey>,
): Partial<Record<SourceKey, T>> {
  let next = record
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(next, key)) continue
    if (next === record) next = { ...next }
    delete next[key]
  }
  return next
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
