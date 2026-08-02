import { useCallback, useEffect, useRef, useState } from "react"

import type { MarkEntriesReadRequest } from "../api/reader.generated"
import { isAbortError, isUnauthenticatedError, readerErrorMessage } from "./controllerErrors"
import type { ReaderApi } from "./controllerApi"
import type { ReaderSession, SessionTask } from "./controllerSession"
import type { ReaderAction } from "./reducer"
import { sourceKey, type ReaderState, type SourceKey } from "./types"

interface BulkReadOptions {
  api: ReaderApi
  csrfToken: string
  dispatch: (action: ReaderAction) => void
  stateRef: { current: ReaderState }
  session: ReaderSession
  userId?: string
  onReconciliationStarted: () => number
  onReconciliationFinished: (
    revalidationId: number,
    validatedSourceKey: SourceKey | null,
  ) => void
  reloadSubscriptions: () => Promise<boolean>
  replaceEntries: () => Promise<boolean>
  reloadSelectedEntry: () => Promise<boolean>
  runReadAction: <T>(operation: () => Promise<T>) => Promise<T>
}

interface BulkReadOperation {
  request: MarkEntriesReadRequest
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

export function useBulkReadActions({
  api,
  csrfToken,
  dispatch,
  stateRef,
  session,
  userId,
  onReconciliationStarted,
  onReconciliationFinished,
  reloadSubscriptions,
  replaceEntries,
  reloadSelectedEntry,
  runReadAction,
}: BulkReadOptions) {
  const [isMarkingRead, setIsMarkingRead] = useState(false)
  const isMarkingReadRef = useRef(false)
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])
  const runMarkRead = useCallback(async (
    operationFactory: (task: SessionTask) => Promise<BulkReadOperation | null>,
  ): Promise<boolean> => {
    if (isMarkingReadRef.current) return false
    isMarkingReadRef.current = true
    setIsMarkingRead(true)
    try {
      return await runReadAction(async () => {
        const task = session.begin()
        if (!task) return false
        try {
          const operation = await operationFactory(task)
          if (!operation || !session.isCurrent(task)) return false
          await api.markEntriesRead(operation.request, csrfToken, task.controller.signal)
          if (!session.isCurrent(task)) return false
          const revalidationId = onReconciliationStarted()
          dispatch({
            type: "bulkReadCommitted",
            entryIds: operation.entryIds,
            affectedFeedIds: operation.affectedFeedIds,
            retainedSourceKey: operation.retainedSourceKey,
            retainedQueueEntryIds: operation.retainedQueueEntryIds,
            retainedPendingEntryIds: operation.retainedPendingEntryIds,
            retainedQueueGeneration: operation.retainedQueueGeneration,
            retainedSnapshotGeneration: operation.retainedSnapshotGeneration,
            retainedPendingSnapshotGeneration:
              operation.retainedPendingSnapshotGeneration,
            invalidateAllSources: operation.invalidateAllSources,
          })
          void (async () => {
            let validatedSourceKey: SourceKey | null = null
            try {
              if (!await reloadSubscriptions()) return
              const selectedSourceKey = sourceKey(stateRef.current.selectedSource)
              const [sourceResult] = await Promise.allSettled([
                replaceEntries(),
                reloadSelectedEntry(),
              ])
              if (
                sourceResult.status === "fulfilled" &&
                sourceResult.value &&
                sourceKey(stateRef.current.selectedSource) === selectedSourceKey
              ) {
                validatedSourceKey = selectedSourceKey
              }
            } catch {
              validatedSourceKey = null
            } finally {
              onReconciliationFinished(revalidationId, validatedSourceKey)
            }
          })()
          return true
        } catch (error) {
          if (isAbortError(error)) return false
          if (!session.isCurrent(task)) return false
          if (isUnauthenticatedError(error)) {
            await session.expire(task)
            return false
          }
          dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
          return false
        } finally {
          session.finish(task)
        }
      })
    } finally {
      isMarkingReadRef.current = false
      if (mountedRef.current) setIsMarkingRead(false)
    }
  }, [
    api,
    csrfToken,
    dispatch,
    onReconciliationFinished,
    onReconciliationStarted,
    reloadSelectedEntry,
    reloadSubscriptions,
    replaceEntries,
    runReadAction,
    session,
  ])

  const markCurrentSourceRead = useCallback(() => {
    const selectedSourceKey = sourceKey(stateRef.current.selectedSource)
    return runMarkRead(async () => {
      const state = stateRef.current
      if (sourceKey(state.selectedSource) !== selectedSourceKey) return null
      return markReadOperation(state)
    })
  }, [runMarkRead, stateRef])
  const markFeedRead = useCallback(
    (feedId: string) => runMarkRead(async (task) => {
      const stateAtRequest = stateRef.current
      const retainedSourceKey = sourceKey(stateAtRequest.selectedSource)
      const entryIds = new Set<string>()
      for (const entry of Object.values(stateAtRequest.entriesById)) {
        if (entry.feedId === feedId) entryIds.add(entry.entryId)
      }
      for (const detail of Object.values(stateAtRequest.detailsById)) {
        if (detail.feedId === feedId) entryIds.add(detail.entryId)
      }
      const page = await api.listEntries({
        feedId,
        state: "ALL",
        limit: 1,
        signal: task.controller.signal,
      })
      if (userId !== undefined && page.ownerUserId !== userId) {
        await session.expire(task)
        return null
      }
      for (const entry of page.items) entryIds.add(entry.entryId)
      return {
        request: { snapshotGeneration: page.snapshotGeneration, feedId },
        entryIds: [...entryIds],
        affectedFeedIds: [feedId],
        retainedSourceKey,
        retainedQueueEntryIds:
          stateAtRequest.queueBySourceKey[retainedSourceKey]?.slice() ?? null,
        retainedPendingEntryIds:
          stateAtRequest.pendingNewEntriesBySource[retainedSourceKey]?.slice() ?? null,
        retainedQueueGeneration: stateAtRequest.requestGenerationByPane.queue,
        retainedSnapshotGeneration:
          stateAtRequest.snapshotGenerationBySource[retainedSourceKey] ?? null,
        retainedPendingSnapshotGeneration:
          stateAtRequest.pendingSnapshotGenerationBySource[retainedSourceKey] ?? null,
        invalidateAllSources: false,
      }
    }),
    [api, runMarkRead, session, userId],
  )

  return { isMarkingRead, markCurrentSourceRead, markFeedRead }
}

function markReadOperation(state: ReaderState): BulkReadOperation | null {
  const source = state.selectedSource
  if (
    state.feedSearchQuery ||
    (source.kind === "smart" && source.state === "STARRED")
  ) {
    return null
  }
  const snapshotGeneration = state.snapshotGenerationBySource[sourceKey(source)]
  if (snapshotGeneration === undefined) return null
  const key = sourceKey(source)
  const entryIds = [...(state.queueBySourceKey[key] ?? [])]
  const affectedFeedIds = feedIdsForSource(state, source)
  let request: MarkEntriesReadRequest
  switch (source.kind) {
    case "smart":
      request = { snapshotGeneration }
      break
    case "feed":
      request = { snapshotGeneration, feedId: source.feedId }
      break
    case "category":
      request = { snapshotGeneration, categoryId: source.categoryId }
      break
  }
  return {
    request,
    entryIds,
    affectedFeedIds,
    retainedSourceKey: key,
    retainedQueueEntryIds: entryIds,
    retainedPendingEntryIds:
      state.pendingNewEntriesBySource[key]?.slice() ?? null,
    retainedQueueGeneration: state.requestGenerationByPane.queue,
    retainedSnapshotGeneration: snapshotGeneration,
    retainedPendingSnapshotGeneration:
      state.pendingSnapshotGenerationBySource[key] ?? null,
    invalidateAllSources: source.kind !== "feed",
  }
}

function feedIdsForSource(
  state: ReaderState,
  source: ReaderState["selectedSource"],
): string[] {
  if (source.kind === "feed") return [source.feedId]
  const feedIds = new Set<string>()
  for (const subscriptionId of state.subscriptionOrder) {
    const subscription = state.subscriptionsById[subscriptionId]
    if (!subscription) continue
    if (source.kind === "category" && subscription.categoryId !== source.categoryId) {
      continue
    }
    feedIds.add(subscription.feedId)
  }
  return [...feedIds]
}
