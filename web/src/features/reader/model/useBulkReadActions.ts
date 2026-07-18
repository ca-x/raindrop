import { useCallback, useState } from "react"

import type { MarkEntriesReadRequest } from "../api/reader.generated"
import { isAbortError, isUnauthenticatedError, readerErrorMessage } from "./controllerErrors"
import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
import type { ReaderAction } from "./reducer"
import { sourceKey, type ReaderState } from "./types"

interface BulkReadOptions {
  api: ReaderApi
  csrfToken: string
  dispatch: (action: ReaderAction) => void
  stateRef: { current: ReaderState }
  session: ReaderSession
  reloadSubscriptions: () => Promise<void>
  replaceEntries: () => Promise<void>
}

export function useBulkReadActions({
  api,
  csrfToken,
  dispatch,
  stateRef,
  session,
  reloadSubscriptions,
  replaceEntries,
}: BulkReadOptions) {
  const [isMarkingRead, setIsMarkingRead] = useState(false)
  const markCurrentSourceRead = useCallback(async (): Promise<boolean> => {
    const state = stateRef.current
    const request = markReadRequest(state)
    if (!request || isMarkingRead) return false
    const task = session.begin()
    if (!task) return false
    setIsMarkingRead(true)
    try {
      await api.markEntriesRead(request, csrfToken, task.controller.signal)
      if (!session.isCurrent(task)) return false
      await Promise.all([reloadSubscriptions(), replaceEntries()])
      return session.isCurrent(task)
    } catch (error) {
      if (isAbortError(error)) return false
      if (!session.isCurrent(task)) return false
      if (isUnauthenticatedError(error)) {
        session.expire(task)
        return false
      }
      dispatch({ type: "mutationErrorSet", error: readerErrorMessage(error) })
      return false
    } finally {
      session.finish(task)
      setIsMarkingRead(false)
    }
  }, [
    api,
    csrfToken,
    dispatch,
    isMarkingRead,
    reloadSubscriptions,
    replaceEntries,
    session,
    stateRef,
  ])

  return { isMarkingRead, markCurrentSourceRead }
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
