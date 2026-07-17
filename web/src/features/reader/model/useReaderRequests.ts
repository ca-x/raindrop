import { useCallback, useEffect, useRef } from "react"

import type { ListEntriesOptions } from "../api/entries"
import type { Subscription } from "../api/subscription.generated"
import type { ReaderApi } from "./controllerApi"
import type { ReaderSession } from "./controllerSession"
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
  session: ReaderSession
}

type Pane = "subscriptions" | "queue" | "detail"

export function useReaderRequests({
  api,
  dispatch,
  stateRef,
  session,
}: ReaderRequestOptions) {
  const controllers = useRef<Partial<Record<Pane, AbortController>>>({})

  const beginRequest = useCallback(
    (pane: Pane) => {
      controllers.current[pane]?.abort()
      const task = session.begin()
      if (!task) return null
      controllers.current[pane] = task.controller
      const generation = stateRef.current.requestGenerationByPane[pane] + 1
      return { ...task, generation }
    },
    [session, stateRef],
  )

  const loadSubscriptions = useCallback(async () => {
    const request = beginRequest("subscriptions")
    if (!request) return
    const { controller, generation } = request
    const current = () =>
      session.isCurrent(request) &&
      stateRef.current.requestGenerationByPane.subscriptions === generation
    dispatch({ type: "subscriptionsRequested", generation })
    try {
      const subscriptions: Subscription[] = []
      let cursor: string | undefined
      do {
        const page = await api.listSubscriptions({ cursor, signal: controller.signal })
        if (!current()) return
        subscriptions.push(...page.items)
        cursor = page.nextCursor ?? undefined
      } while (cursor !== undefined)
      if (current()) dispatch({ type: "subscriptionsReceived", generation, subscriptions })
    } catch (error) {
      if (isAbortError(error)) return
      if (!current()) return
      if (isUnauthenticatedError(error)) return session.expire(request)
      dispatch({
        type: "subscriptionsFailed",
        generation,
        error: readerErrorMessage(error),
      })
    } finally {
      session.finish(request)
    }
  }, [api, beginRequest, dispatch, session, stateRef])

  const loadSource = useCallback(
    async (source: ReaderSource, mode: "replace" | "discover") => {
      const request = beginRequest("queue")
      if (!request) return
      const { controller, generation } = request
      const current = () =>
        session.isCurrent(request) &&
        stateRef.current.requestGenerationByPane.queue === generation &&
        sameSource(stateRef.current.selectedSource, source)
      dispatch({ type: "sourceRequested", source, generation })
      try {
        const page = await api.listEntries({
          ...entryListOptions(source),
          signal: controller.signal,
        })
        if (!current()) return
        dispatch({
          type: "sourceReceived",
          source,
          generation,
          entries: page.items,
          mode,
        })
      } catch (error) {
        if (isAbortError(error)) return
        if (!current()) return
        if (isUnauthenticatedError(error)) return session.expire(request)
        dispatch({
          type: "sourceFailed",
          source,
          generation,
          error: readerErrorMessage(error),
        })
      } finally {
        session.finish(request)
      }
    },
    [api, beginRequest, dispatch, session, stateRef],
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
      if (!session.active()) return
      controllers.current.detail?.abort()
      dispatch({ type: "sourceSelected", source })
      await loadSource(source, "replace")
    },
    [dispatch, loadSource, session],
  )

  const selectEntry = useCallback(
    async (entryId: string | null) => {
      if (!session.active()) return
      dispatch({ type: "entrySelected", entryId })
      if (entryId === null) {
        controllers.current.detail?.abort()
        return
      }
      const request = beginRequest("detail")
      if (!request) return
      const { controller, generation } = request
      const current = () =>
        session.isCurrent(request) &&
        stateRef.current.requestGenerationByPane.detail === generation &&
        stateRef.current.selectedEntryId === entryId
      dispatch({ type: "detailRequested", entryId, generation })
      try {
        const detail = await api.getEntry(entryId, controller.signal)
        if (current()) dispatch({ type: "detailReceived", entryId, generation, detail })
      } catch (error) {
        if (isAbortError(error)) return
        if (!current()) return
        if (isUnauthenticatedError(error)) return session.expire(request)
        dispatch({
          type: "detailFailed",
          entryId,
          generation,
          error: readerErrorMessage(error),
        })
      } finally {
        session.finish(request)
      }
    },
    [api, beginRequest, dispatch, session, stateRef],
  )

  const reloadEntries = useCallback(
    () => loadSource(stateRef.current.selectedSource, "discover"),
    [loadSource, stateRef],
  )

  const mergePendingEntries = useCallback(() => {
    if (!session.active()) return
    dispatch({ type: "pendingEntriesMerged", source: stateRef.current.selectedSource })
  }, [dispatch, session, stateRef])

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

function sameSource(left: ReaderSource, right: ReaderSource): boolean {
  if (left.kind === "feed" && right.kind === "feed") return left.feedId === right.feedId
  if (left.kind === "smart" && right.kind === "smart") return left.state === right.state
  return false
}
