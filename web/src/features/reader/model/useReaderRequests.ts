import { useCallback, useEffect, useRef } from "react"

import type { ListEntriesOptions } from "../api/entries"
import type { Subscription } from "../api/subscription.generated"
import type { ReaderApi } from "./controllerApi"
import {
  isAbortError,
  isUnauthenticatedError,
  readerErrorMessage,
} from "./controllerErrors"
import type { ReaderAction } from "./reducer"
import type { ReaderSource, ReaderState } from "./types"

interface ReaderRequestOptions {
  api: ReaderApi
  dispatch: (action: ReaderAction) => void
  stateRef: { current: ReaderState }
  expireSession: () => void
}

type Pane = "subscriptions" | "queue" | "detail"

export function useReaderRequests({
  api,
  dispatch,
  stateRef,
  expireSession,
}: ReaderRequestOptions) {
  const controllers = useRef<Partial<Record<Pane, AbortController>>>({})

  const beginRequest = useCallback(
    (pane: Pane) => {
      controllers.current[pane]?.abort()
      const controller = new AbortController()
      controllers.current[pane] = controller
      const generation = stateRef.current.requestGenerationByPane[pane] + 1
      return { controller, generation }
    },
    [stateRef],
  )

  const loadSubscriptions = useCallback(async () => {
    const { controller, generation } = beginRequest("subscriptions")
    dispatch({ type: "subscriptionsRequested", generation })
    try {
      const subscriptions: Subscription[] = []
      let cursor: string | undefined
      do {
        const page = await api.listSubscriptions({ cursor, signal: controller.signal })
        subscriptions.push(...page.items)
        cursor = page.nextCursor ?? undefined
      } while (cursor !== undefined)
      dispatch({ type: "subscriptionsReceived", generation, subscriptions })
    } catch (error) {
      if (isAbortError(error)) return
      if (isUnauthenticatedError(error)) return expireSession()
      dispatch({
        type: "subscriptionsFailed",
        generation,
        error: readerErrorMessage(error),
      })
    }
  }, [api, beginRequest, dispatch, expireSession])

  const loadSource = useCallback(
    async (source: ReaderSource, mode: "replace" | "discover") => {
      const { controller, generation } = beginRequest("queue")
      dispatch({ type: "sourceRequested", source, generation })
      try {
        const page = await api.listEntries({
          ...entryListOptions(source),
          signal: controller.signal,
        })
        dispatch({
          type: "sourceReceived",
          source,
          generation,
          entries: page.items,
          mode,
        })
      } catch (error) {
        if (isAbortError(error)) return
        if (isUnauthenticatedError(error)) return expireSession()
        dispatch({
          type: "sourceFailed",
          source,
          generation,
          error: readerErrorMessage(error),
        })
      }
    },
    [api, beginRequest, dispatch, expireSession],
  )

  const load = useCallback(
    async () => {
      await Promise.all([
        loadSubscriptions(),
        loadSource(stateRef.current.selectedSource, "replace"),
      ])
    },
    [loadSource, loadSubscriptions, stateRef],
  )

  const selectSource = useCallback(
    async (source: ReaderSource) => {
      controllers.current.detail?.abort()
      dispatch({ type: "sourceSelected", source })
      await loadSource(source, "replace")
    },
    [dispatch, loadSource],
  )

  const selectEntry = useCallback(
    async (entryId: string | null) => {
      dispatch({ type: "entrySelected", entryId })
      if (entryId === null) {
        controllers.current.detail?.abort()
        return
      }
      const { controller, generation } = beginRequest("detail")
      dispatch({ type: "detailRequested", entryId, generation })
      try {
        const detail = await api.getEntry(entryId, controller.signal)
        dispatch({ type: "detailReceived", entryId, generation, detail })
      } catch (error) {
        if (isAbortError(error)) return
        if (isUnauthenticatedError(error)) return expireSession()
        dispatch({
          type: "detailFailed",
          entryId,
          generation,
          error: readerErrorMessage(error),
        })
      }
    },
    [api, beginRequest, dispatch, expireSession],
  )

  const reloadEntries = useCallback(
    () => loadSource(stateRef.current.selectedSource, "discover"),
    [loadSource, stateRef],
  )

  const mergePendingEntries = useCallback(() => {
    dispatch({ type: "pendingEntriesMerged", source: stateRef.current.selectedSource })
  }, [dispatch, stateRef])

  useEffect(
    () => () => {
      for (const controller of Object.values(controllers.current)) controller.abort()
    },
    [],
  )

  return { load, selectSource, selectEntry, reloadEntries, mergePendingEntries }
}

function entryListOptions(source: ReaderSource): ListEntriesOptions {
  return source.kind === "feed"
    ? { feedId: source.feedId, state: "ALL" }
    : { state: source.state }
}
