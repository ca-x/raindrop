import { useCallback, useEffect, useRef, useState } from "react"

import type { MarkEntriesReadRequest } from "../api/reader.generated"
import { isAbortError, isUnauthenticatedError, readerErrorMessage } from "./controllerErrors"
import type { ReaderApi } from "./controllerApi"
import type { ReaderSession, SessionTask } from "./controllerSession"
import type { ReaderAction } from "./reducer"
import { sourceKey, type ReaderState } from "./types"

interface BulkReadOptions {
  api: ReaderApi
  csrfToken: string
  dispatch: (action: ReaderAction) => void
  stateRef: { current: ReaderState }
  session: ReaderSession
  userId?: string
  reloadSubscriptions: () => Promise<void>
  replaceEntries: () => Promise<void>
}

export function useBulkReadActions({
  api,
  csrfToken,
  dispatch,
  stateRef,
  session,
  userId,
  reloadSubscriptions,
  replaceEntries,
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
    requestFactory: (task: SessionTask) => Promise<MarkEntriesReadRequest | null>,
  ): Promise<boolean> => {
    if (isMarkingReadRef.current) return false
    const task = session.begin()
    if (!task) return false
    isMarkingReadRef.current = true
    setIsMarkingRead(true)
    try {
      const request = await requestFactory(task)
      if (!request || !session.isCurrent(task)) return false
      await api.markEntriesRead(request, csrfToken, task.controller.signal)
      if (!session.isCurrent(task)) return false
      await Promise.all([reloadSubscriptions(), replaceEntries()])
      return session.isCurrent(task)
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
      isMarkingReadRef.current = false
      if (mountedRef.current) setIsMarkingRead(false)
    }
  }, [
    api,
    csrfToken,
    dispatch,
    reloadSubscriptions,
    replaceEntries,
    session,
  ])

  const markCurrentSourceRead = useCallback(
    () => runMarkRead(async () => markReadRequest(stateRef.current)),
    [runMarkRead, stateRef],
  )
  const markFeedRead = useCallback(
    (feedId: string) => runMarkRead(async (task) => {
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
      return { snapshotGeneration: page.snapshotGeneration, feedId }
    }),
    [api, runMarkRead, session, userId],
  )

  return { isMarkingRead, markCurrentSourceRead, markFeedRead }
}

function markReadRequest(state: ReaderState): MarkEntriesReadRequest | null {
  const source = state.selectedSource
  if (
    state.feedSearchQuery ||
    (source.kind === "smart" && source.state === "STARRED")
  ) {
    return null
  }
  const snapshotGeneration = state.snapshotGenerationBySource[sourceKey(source)]
  if (snapshotGeneration === undefined) return null
  switch (source.kind) {
    case "smart":
      return { snapshotGeneration }
    case "feed":
      return { snapshotGeneration, feedId: source.feedId }
    case "category":
      return { snapshotGeneration, categoryId: source.categoryId }
  }
}
